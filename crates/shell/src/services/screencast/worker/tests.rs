use super::*;
use std::{
    os::unix::fs::PermissionsExt,
    sync::{Arc, atomic::AtomicU64, mpsc},
};
use way_shell_core::capture::{CaptureOutput, Rect};

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "way-shell-recorder-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn script(&self, name: &str, body: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }
    fn programs(&self, recorder: &str, audio: bool) -> Programs {
        Programs {
            recorder: self.script("recorder", recorder),
            ffmpeg: self.script("ffmpeg", "case \"$*\" in *libx264*) exit 1;; esac\nexit 0"),
            ffprobe: self.script("ffprobe", &format!("printf '%s' '{}'", if audio {
                r#"{"streams":[{"codec_type":"video","codec_name":"h264"},{"codec_type":"audio","codec_name":"aac"}],"format":{"duration":"1.2"}}"#
            } else { r#"{"streams":[{"codec_type":"video","codec_name":"h264"}],"format":{"duration":"1.2"}}"# })),
        }
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn target() -> Target {
    Target {
        output: CaptureOutput {
            connector: "DP-1".into(),
            bounds: Rect {
                x: -200.0,
                y: 0.0,
                width: 200.0,
                height: 100.0,
            },
            scale: 1.25,
            transform: "normal".into(),
            native_size: (250, 125),
        },
        region: Some(Rect {
            x: -150.0,
            y: 10.0,
            width: 81.0,
            height: 51.0,
        }),
    }
}
fn wait_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !path.exists() {
        assert!(Instant::now() < deadline, "recorder did not become ready");
        thread::sleep(POLL);
    }
}
fn graceful_program(directory: &Directory, audio: bool) -> Programs {
    directory.programs(
        &format!(
            r#"
file=
while [ "$#" -gt 0 ]; do
  if [ "$1" = -f ]; then shift; file=$1; fi
  shift
done
trap 'printf encoded > "$file"; exit 0' INT
echo $$ > '{}'
while :; do sleep 0.025; done
"#,
            directory.0.join("ready").display()
        ),
        audio,
    )
}

#[test]
fn graceful_stop_finalizes_without_overwriting_and_reaps_child() {
    let directory = Directory::new();
    let programs = graceful_program(&directory, true);
    let existing = reserve(&directory.0, "same").unwrap();
    fs::write(&existing, b"existing").unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let request_stop = stop.clone();
    let path = directory.0.clone();
    let (sender, receiver) = mpsc::channel();
    let worker = thread::spawn(move || {
        record(
            &programs,
            &path,
            "same",
            &target(),
            Audio::Desktop,
            &stop,
            &sender,
        )
    });
    assert!(matches!(
        receiver.recv_timeout(Duration::from_secs(2)).unwrap(),
        Event::Started(_)
    ));
    wait_file(&directory.0.join("ready"));
    request_stop.store(true, Ordering::Release);
    request_stop.store(true, Ordering::Release);
    let saved = worker.join().unwrap().unwrap().unwrap();
    assert_ne!(saved, existing);
    assert_eq!(fs::read(existing).unwrap(), b"existing");
    assert_eq!(fs::read(saved).unwrap(), b"encoded");
    let pid = fs::read_to_string(directory.0.join("ready")).unwrap();
    assert!(!Path::new(&format!("/proc/{}", pid.trim())).exists());
}

#[test]
fn missing_audio_is_reported_and_nonempty_video_is_retained() {
    let directory = Directory::new();
    let programs = graceful_program(&directory, false);
    let stop = Arc::new(AtomicBool::new(false));
    let request_stop = stop.clone();
    let path = directory.0.clone();
    let (sender, _) = mpsc::channel();
    let worker = thread::spawn(move || {
        record(
            &programs,
            &path,
            "audio",
            &target(),
            Audio::Microphone,
            &stop,
            &sender,
        )
    });
    wait_file(&directory.0.join("ready"));
    request_stop.store(true, Ordering::Release);
    let error = worker.join().unwrap().unwrap_err();
    assert!(
        error.contains("requested audio was not captured"),
        "{error}"
    );
    assert!(error.contains("retained"));
    assert_eq!(
        fs::read(directory.0.join("Screencast-audio.mp4")).unwrap(),
        b"encoded"
    );
}

#[test]
fn cancelled_preparation_missing_program_and_save_errors_leave_no_recording() {
    let directory = Directory::new();
    let mut programs = directory.programs("exit 1", false);
    let (sender, _) = mpsc::channel();
    assert_eq!(
        record(
            &programs,
            &directory.0,
            "cancel",
            &target(),
            Audio::Off,
            &AtomicBool::new(true),
            &sender
        )
        .unwrap(),
        None
    );
    programs.recorder = directory.0.join("missing");
    assert!(
        record(
            &programs,
            &directory.0,
            "missing",
            &target(),
            Audio::Off,
            &AtomicBool::new(false),
            &sender
        )
        .is_err()
    );
    assert!(!directory.0.join("Screencast-missing.mp4").exists());
    let blocker = directory.0.join("file");
    fs::write(&blocker, "file").unwrap();
    assert!(reserve(&blocker, "bad").is_err());
}

#[test]
fn unexpected_exit_and_stop_timeout_retain_partial_files_and_reap() {
    let directory = Directory::new();
    let programs = directory.programs(r#"while [ "$#" -gt 0 ]; do if [ "$1" = -f ]; then shift; printf partial > "$1"; fi; shift; done
exit 1"#, false);
    let (sender, _) = mpsc::channel();
    let error = record(
        &programs,
        &directory.0,
        "crash",
        &target(),
        Audio::Off,
        &AtomicBool::new(false),
        &sender,
    )
    .unwrap_err();
    assert!(error.contains("exited"));
    assert!(error.contains("retained"));
    let programs = directory.programs(
        &format!(
            "trap '' INT\necho $$ > '{}'\nexec sleep 30",
            directory.0.join("ready").display()
        ),
        false,
    );
    let stop = Arc::new(AtomicBool::new(false));
    let request_stop = stop.clone();
    let path = directory.0.clone();
    let (sender, _) = mpsc::channel();
    let worker = thread::spawn(move || {
        record(
            &programs,
            &path,
            "stuck",
            &target(),
            Audio::Off,
            &stop,
            &sender,
        )
    });
    wait_file(&directory.0.join("ready"));
    request_stop.store(true, Ordering::Release);
    assert!(worker.join().unwrap().unwrap_err().contains("five seconds"));
    let pid = fs::read_to_string(directory.0.join("ready")).unwrap();
    assert!(!Path::new(&format!("/proc/{}", pid.trim())).exists());
    assert!(!directory.0.join("Screencast-stuck.mp4").exists());
}

#[test]
fn command_preserves_geometry_paths_and_explicit_audio_sources() {
    let directory = Directory::new();
    let programs = directory.programs("exit 0", false);
    let path = directory.0.join("a recording ' $() .mp4");
    for (audio, expected) in [
        (Audio::Off, None),
        (Audio::Microphone, Some("--audio=@DEFAULT_SOURCE@")),
        (Audio::Desktop, Some("--audio=@DEFAULT_MONITOR@")),
    ] {
        let command = command(&programs, &target(), audio, "libopenh264", &path);
        let args: Vec<_> = command.get_args().collect();
        assert!(args.contains(&path.as_os_str()));
        assert!(args.contains(&std::ffi::OsStr::new("-150,10 81x51")));
        assert_eq!(
            args.iter()
                .filter(|a| a.to_string_lossy().starts_with("--audio="))
                .count(),
            usize::from(expected.is_some())
        );
        if let Some(expected) = expected {
            assert!(args.contains(&std::ffi::OsStr::new(expected)));
        }
    }
}
