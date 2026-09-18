use std::{
    collections::HashSet,
    os::{linux::net::SocketAddrExt, unix::net::UnixDatagram},
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

struct Server {
    runtime: PathBuf,
    socket: UnixDatagram,
}

impl Server {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let runtime = std::env::temp_dir().join(format!(
            "way-sh-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&runtime).unwrap();
        let socket = UnixDatagram::bind(runtime.join("way-shell.sock")).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        Self { runtime, socket }
    }

    fn exchange(&self, args: &[&str], response: &[u8]) -> (Vec<u8>, Output) {
        let child = Running::new(args, Some(&self.runtime));
        let mut bytes = [0; 64];
        let (size, address) = self.socket.recv_from(&mut bytes).unwrap();
        self.socket.send_to_addr(response, &address).unwrap();
        (bytes[..size].to_vec(), child.finish())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.runtime);
    }
}

struct Running(Option<Child>);

impl Running {
    fn new(args: &[&str], runtime: Option<&Path>) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_way-sh"));
        command
            .args(args)
            .env_remove("XDG_RUNTIME_DIR")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(runtime) = runtime {
            command.env("XDG_RUNTIME_DIR", runtime);
        }
        Self(Some(command.spawn().unwrap()))
    }

    fn finish(mut self) -> Output {
        let deadline = Instant::now() + Duration::from_secs(4);
        while self.0.as_mut().unwrap().try_wait().unwrap().is_none() {
            assert!(
                Instant::now() < deadline,
                "CLI failed to finish within four seconds"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        self.0.take().unwrap().wait_with_output().unwrap()
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn status(output: Output, expected: i32) {
    assert_eq!(
        output.status.code(),
        Some(expected),
        "stdout: {}; stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn every_c_command_keeps_its_wire_encoding() {
    let server = Server::new();
    let fixtures: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/cli.json")).unwrap();
    for fixture in fixtures.as_array().unwrap() {
        let args: Vec<_> = fixture["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|arg| arg.as_str().unwrap())
            .collect();
        let mut expected = (fixture["opcode"].as_u64().unwrap() as u32)
            .to_le_bytes()
            .to_vec();
        if let Some(volume) = fixture["volume"].as_f64() {
            expected.extend((volume as f32).to_le_bytes());
        }
        let (bytes, output) = server.exchange(&args, &[1, 0, 0, 0]);
        assert_eq!(bytes, expected, "{args:?}");
        status(output, 0);
    }
}

#[test]
fn help_needs_no_shell() {
    for args in [
        &[][..],
        &["--help"],
        &["-h"],
        &["volume"],
        &["volume", "--help"],
        &["volume", "set", "--help"],
    ] {
        let output = Running::new(args, None).finish();
        assert!(!output.stdout.is_empty());
        status(output, 0);
    }
}

#[test]
fn invalid_arguments_are_rejected_before_connecting() {
    for args in [
        &["unknown"][..],
        &["volume", "unknown"],
        &["volume", "set"],
        &["volume", "up", "extra"],
        &["volume", "set", "0", "1"],
    ] {
        status(Running::new(args, None).finish(), 2);
    }
    for value in [
        "", "abc", "0.5oops", "NaN", "inf", "-inf", "-0.1", "1.1", "1e999", "1e-999",
    ] {
        status(Running::new(&["volume", "set", value], None).finish(), 2);
    }
}

#[test]
fn unavailable_servers_and_invalid_paths_fail() {
    status(Running::new(&["volume", "up"], None).finish(), 1);
    for path in [
        "".into(),
        "/does/not/exist".into(),
        format!("/{}", "x".repeat(200)),
        "relative".into(),
    ] {
        status(
            Running::new(&["volume", "up"], Some(Path::new(&path))).finish(),
            1,
        );
    }
    let server = Server::new();
    std::fs::remove_file(server.runtime.join("way-shell.sock")).unwrap();
    std::fs::write(server.runtime.join("way-shell.sock"), "not a socket").unwrap();
    status(
        Running::new(&["volume", "up"], Some(&server.runtime)).finish(),
        1,
    );
}

#[test]
fn failure_and_malformed_replies_fail() {
    let server = Server::new();
    for response in [
        &[0, 0, 0, 0][..],
        &[],
        &[1],
        &[1, 0, 0, 0, 0],
        &[2, 0, 0, 0],
    ] {
        status(server.exchange(&["volume", "up"], response).1, 1);
    }
}

#[test]
fn concurrent_clients_have_distinct_return_addresses() {
    let server = Server::new();
    let children: Vec<_> = (0..8)
        .map(|_| Running::new(&["volume", "up"], Some(&server.runtime)))
        .collect();
    let mut addresses = HashSet::new();
    for _ in &children {
        let mut bytes = [0; 64];
        let (size, address) = server.socket.recv_from(&mut bytes).unwrap();
        assert_eq!(&bytes[..size], &[2, 0, 0, 0]);
        assert!(addresses.insert(address.as_abstract_name().unwrap().to_vec()));
        server.socket.send_to_addr(&[1, 0, 0, 0], &address).unwrap();
    }
    for child in children {
        status(child.finish(), 0);
    }
}

#[test]
fn response_deadline_is_two_seconds() {
    let server = Server::new();
    let start = Instant::now();
    let child = Running::new(&["volume", "up"], Some(&server.runtime));
    server.socket.recv_from(&mut [0; 64]).unwrap();
    status(child.finish(), 1);
    assert!(start.elapsed() >= Duration::from_millis(1800));
    assert!(start.elapsed() < Duration::from_secs(4));
}

#[test]
fn shortcut_commands_append_wire_opcodes() {
    let server = Server::new();
    for (action, opcode) in [("show", 34u32), ("hide", 35), ("toggle", 36)] {
        let (bytes, output) = server.exchange(&["shortcuts", action], &[1, 0, 0, 0]);
        assert_eq!(bytes, opcode.to_le_bytes());
        status(output, 0);
    }
}
