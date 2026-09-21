//! The command-line client, independent of GTK and desktop services.
use clap::{Arg, Command};
use std::{
    io,
    os::{
        linux::net::SocketAddrExt,
        unix::{
            fs::FileTypeExt,
            net::{SocketAddr, UnixDatagram},
        },
    },
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use way_shell_core::{
    IPC_SOCKET_NAME,
    ipc::{Request, decode_response},
};

pub const RESPONSE_TIMEOUT: Duration = Duration::from_secs(2);
type CommandGroup = (&'static str, &'static [(&'static str, u32)]);
const GROUPS: &[CommandGroup] = &[
    ("message-tray", &[("open", 1)]),
    ("volume", &[("up", 2), ("down", 3), ("set", 4), ("mute", 5)]),
    (
        "brightness",
        &[
            ("up", 6),
            ("down", 7),
            ("keyboard-up", 29),
            ("keyboard-down", 30),
        ],
    ),
    (
        "theme",
        &[
            ("dark", 8),
            ("light", 9),
            ("dump-dark", 10),
            ("dump-light", 11),
        ],
    ),
    ("activities", &[("show", 12), ("hide", 13), ("toggle", 14)]),
    (
        "app-switcher",
        &[("show", 15), ("hide", 16), ("toggle", 17)],
    ),
    (
        "workspace-switcher",
        &[("show", 18), ("hide", 19), ("toggle", 20)],
    ),
    (
        "output-switcher",
        &[("show", 21), ("hide", 22), ("toggle", 23)],
    ),
    (
        "workspace-app-switcher",
        &[("show", 24), ("hide", 25), ("toggle", 26)],
    ),
    ("bluelight-filter", &[("enable", 27), ("disable", 28)]),
    (
        "rename-switcher",
        &[("show", 31), ("hide", 32), ("toggle", 33)],
    ),
];

fn volume(text: &str) -> Result<f32, String> {
    let value: f32 = text.parse().map_err(|_| "expected a decimal volume")?;
    let mantissa = text.split(['e', 'E']).next().unwrap_or("");
    let underflow = value == 0.0 && mantissa.bytes().any(|b| (b'1'..=b'9').contains(&b));
    if !value.is_finite() || !(0.0..=1.0).contains(&value) || value.is_subnormal() || underflow {
        return Err("volume must be finite and between 0.0 and 1.0".into());
    }
    Ok(value)
}

pub fn command() -> Command {
    let mut root = Command::new("way-sh")
        .about("Control Way Shell")
        .version(env!("CARGO_PKG_VERSION"));
    for &(name, actions) in GROUPS {
        let mut group = Command::new(name);
        for &(action, opcode) in actions {
            let mut child = Command::new(action);
            if opcode == 4 {
                child = child.arg(Arg::new("volume").required(true).value_parser(volume));
            }
            group = group.subcommand(child);
        }
        root = root.subcommand(group);
    }
    root
}

fn client_socket() -> io::Result<UnixDatagram> {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    for _ in 0..8 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos();
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let name = format!("way-shell-{}-{now:x}-{sequence:x}", std::process::id());
        let address = SocketAddr::from_abstract_name(name)?;
        match UnixDatagram::bind_addr(&address) {
            Err(error) if error.kind() == io::ErrorKind::AddrInUse => continue,
            result => return result,
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AddrInUse,
        "cannot allocate a unique client address",
    ))
}

fn remaining(deadline: Instant) -> io::Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "shell response deadline exceeded"))
}

pub fn exchange(request: Request, runtime: &Path, timeout: Duration) -> io::Result<bool> {
    let path = runtime.join(IPC_SOCKET_NAME);
    if !runtime.is_absolute() || path.as_os_str().as_encoded_bytes().len() >= 108 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "XDG_RUNTIME_DIR does not fit an absolute Unix socket path",
        ));
    }
    if !path.metadata()?.file_type().is_socket() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "shell path is not a socket",
        ));
    }
    let client = client_socket()?;
    client.connect(path)?;
    let deadline = Instant::now() + timeout;
    let bytes = request.encode();
    loop {
        client.set_write_timeout(Some(remaining(deadline)?))?;
        match client.send(&bytes) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            result => {
                if result? != bytes.len() {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "incomplete command datagram",
                    ));
                }
                break;
            }
        }
    }
    loop {
        client.set_read_timeout(Some(remaining(deadline)?))?;
        let mut response = [0; 5]; // An excess byte detects truncated oversized replies.
        match client.recv(&mut response) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            result => {
                return decode_response(&response[..result?])
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error));
            }
        }
    }
}

pub fn run() -> u8 {
    let mut root = command();
    let matches = match root.clone().try_get_matches() {
        Ok(matches) => matches,
        Err(error) => {
            let status = error.exit_code() as u8;
            let _ = error.print();
            return status;
        }
    };
    let Some((group, group_matches)) = matches.subcommand() else {
        return u8::from(root.print_help().is_err());
    };
    let Some((action, args)) = group_matches.subcommand() else {
        return u8::from(
            root.find_subcommand_mut(group)
                .unwrap()
                .print_help()
                .is_err(),
        );
    };
    let opcode = GROUPS
        .iter()
        .find(|(name, _)| *name == group)
        .unwrap()
        .1
        .iter()
        .find(|(name, _)| *name == action)
        .unwrap()
        .1;
    let volume = if opcode == 4 {
        args.get_one::<f32>("volume").copied()
    } else {
        None
    };
    let request = Request::new(opcode, volume).expect("clap validated the request");
    let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") else {
        eprintln!("way-sh: XDG_RUNTIME_DIR is not set");
        return 1;
    };
    match exchange(request, Path::new(&runtime), RESPONSE_TIMEOUT) {
        Ok(true) => 0,
        Ok(false) => {
            eprintln!("way-sh: shell could not perform the command");
            1
        }
        Err(error) => {
            eprintln!("way-sh: {error}");
            1
        }
    }
}
