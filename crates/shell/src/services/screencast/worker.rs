//! Blocking process supervision stays off GTK's thread. No media is decoded here.
use super::{Audio, Target};
use std::{
    fs::{self, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const POLL: Duration = Duration::from_millis(25);
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);
const STOP_TIMEOUT: Duration = Duration::from_secs(5);
const DIAGNOSTIC_LIMIT: usize = 64 * 1024;

pub(super) enum Event {
    Started(Instant),
    Finished(Result<Option<PathBuf>, String>),
}

pub(super) struct Programs {
    recorder: PathBuf,
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
}
impl Programs {
    pub fn find() -> Result<Self, String> {
        let find = |name| {
            glib::find_program_in_path(name).ok_or_else(|| {
                format!("Install wf-recorder and FFmpeg tools to record video ({name} unavailable)")
            })
        };
        Ok(Self {
            recorder: find("wf-recorder")?,
            ffmpeg: find("ffmpeg")?,
            ffprobe: find("ffprobe")?,
        })
    }
}

// Draining both pipes prevents subprocess stalls; only a bounded diagnostic tail
// is retained. The owning Process always terminates and reaps its child.
struct Process {
    child: Child,
    stdout: Option<JoinHandle<Vec<u8>>>,
    stderr: Option<JoinHandle<Vec<u8>>>,
}
fn drain(mut pipe: impl Read) -> Vec<u8> {
    let mut result = Vec::new();
    let mut buffer = [0; 4096];
    while let Ok(size) = pipe.read(&mut buffer) {
        if size == 0 {
            break;
        }
        let excess = (result.len() + size).saturating_sub(DIAGNOSTIC_LIMIT);
        result.drain(..excess);
        result.extend_from_slice(&buffer[..size]);
    }
    result
}
impl Process {
    fn spawn(command: &mut Command) -> Result<Self, String> {
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                format!(
                    "Could not start {}: {e}",
                    command.get_program().to_string_lossy()
                )
            })?;
        let stdout = child
            .stdout
            .take()
            .map(|pipe| thread::spawn(move || drain(pipe)));
        let stderr = child
            .stderr
            .take()
            .map(|pipe| thread::spawn(move || drain(pipe)));
        Ok(Self {
            child,
            stdout,
            stderr,
        })
    }
    fn interrupt(&mut self) -> Result<(), String> {
        // An unreaped child retains its PID, so it cannot refer to a reused PID.
        if self.child.try_wait().map_err(|e| e.to_string())?.is_none() {
            let result = unsafe { libc::kill(self.child.id() as libc::pid_t, libc::SIGINT) };
            if result != 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
        }
        Ok(())
    }
    fn output(&mut self) -> Result<(ExitStatus, Vec<u8>, String), String> {
        let status = self.child.wait().map_err(|e| e.to_string())?;
        let stdout = self
            .stdout
            .take()
            .and_then(|t| t.join().ok())
            .unwrap_or_default();
        let stderr = self
            .stderr
            .take()
            .and_then(|t| t.join().ok())
            .unwrap_or_default();
        Ok((
            status,
            stdout,
            String::from_utf8_lossy(&stderr).trim().to_owned(),
        ))
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        for reader in [self.stdout.take(), self.stderr.take()]
            .into_iter()
            .flatten()
        {
            let _ = reader.join();
        }
    }
}

fn probe(command: &mut Command, stop: Option<&AtomicBool>) -> Result<Vec<u8>, String> {
    let mut process = Process::spawn(command)?;
    let deadline = Instant::now() + PROBE_TIMEOUT;
    loop {
        if process
            .child
            .try_wait()
            .map_err(|e| e.to_string())?
            .is_some()
        {
            let (status, stdout, stderr) = process.output()?;
            return if status.success() {
                Ok(stdout)
            } else {
                Err(stderr)
            };
        }
        if stop.is_some_and(|s| s.load(Ordering::Acquire)) {
            return Err("Recording cancelled".into());
        }
        if Instant::now() >= deadline {
            return Err("Media tool exceeded its three-second deadline".into());
        }
        thread::sleep(POLL);
    }
}

fn encoder(programs: &Programs, audio: Audio, stop: &AtomicBool) -> Result<&'static str, String> {
    for encoder in ["libx264", "libopenh264"] {
        if stop.load(Ordering::Acquire) {
            return Err("Recording cancelled".into());
        }
        let mut command = Command::new(&programs.ffmpeg);
        command.args([
            "-v",
            "error",
            "-nostdin",
            "-f",
            "lavfi",
            "-i",
            "color=black:s=64x64:r=30",
        ]);
        if audio != Audio::Off {
            command.args([
                "-f",
                "lavfi",
                "-i",
                "anullsrc=r=48000:cl=stereo",
                "-c:a",
                "aac",
            ]);
        }
        command.args([
            "-t", "0.1", "-c:v", encoder, "-pix_fmt", "yuv420p", "-f", "null", "-",
        ]);
        if probe(&mut command, Some(stop)).is_ok() {
            return Ok(encoder);
        }
    }
    Err("No usable H.264 encoder (or AAC encoder for the selected audio). Install FFmpeg with x264 or OpenH264 support".into())
}

fn reserve(directory: &Path, stamp: &str) -> Result<PathBuf, String> {
    fs::create_dir_all(directory)
        .map_err(|e| format!("Could not create {}: {e}", directory.display()))?;
    for suffix in 0..10_000 {
        let name = if suffix == 0 {
            format!("Screencast-{stamp}.mp4")
        } else {
            format!("Screencast-{stamp}-{suffix}.mp4")
        };
        let path = directory.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("Could not save {}: {e}", path.display())),
        }
    }
    Err("Too many recording filename collisions".into())
}

fn command(
    programs: &Programs,
    target: &Target,
    audio: Audio,
    encoder: &str,
    path: &Path,
) -> Command {
    let mut command = Command::new(&programs.recorder);
    // Keep frames arriving on static desktops too. Besides producing 30 fps,
    // this lets wf-recorder leave its Wayland dispatch when Stop interrupts it.
    command.args([
        "-y",
        "--no-damage",
        "-o",
        &target.output.connector,
        "-c",
        encoder,
        "-r",
        "30",
        "-x",
        "yuv420p",
        "-F",
        "pad=ceil(iw/2)*2:ceil(ih/2)*2",
    ]);
    // Overwrite only the empty file we have exclusively reserved, never an
    // existing recording; stdin is closed so no interactive prompt is possible.
    command.arg("-f").arg(path);
    if let Some(area) = target.region {
        command.arg("-g").arg(format!(
            "{},{} {}x{}",
            area.x as i32, area.y as i32, area.width as u32, area.height as u32
        ));
    }
    if encoder == "libopenh264" {
        command.args(["-p", "b=8000000"]);
    }
    if audio != Audio::Off {
        command.args(["--audio-backend=pulse", "-C", "aac"]);
        command.arg(match audio {
            Audio::Microphone => "--audio=@DEFAULT_SOURCE@",
            Audio::Desktop => "--audio=@DEFAULT_MONITOR@",
            Audio::Off => unreachable!(),
        });
    }
    command
}

fn validate(programs: &Programs, path: &Path, audio: Audio) -> Result<(), String> {
    let bytes = probe(
        Command::new(&programs.ffprobe)
            .args([
                "-v",
                "error",
                "-show_entries",
                "stream=codec_type,codec_name:format=duration",
                "-of",
                "json",
            ])
            .arg(path),
        None,
    )?;
    let json: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("Invalid ffprobe response: {e}"))?;
    let streams = json["streams"]
        .as_array()
        .ok_or("Recording has no media streams")?;
    if !streams
        .iter()
        .any(|s| s["codec_type"] == "video" && s["codec_name"] == "h264")
    {
        return Err("Recording has no H.264 video stream".into());
    }
    if audio != Audio::Off
        && !streams
            .iter()
            .any(|s| s["codec_type"] == "audio" && s["codec_name"] == "aac")
    {
        return Err("The video was saved, but the requested audio was not captured".into());
    }
    if !json["format"]["duration"]
        .as_str()
        .and_then(|v| v.parse::<f64>().ok())
        .is_some_and(|duration| duration.is_finite() && duration > 0.0)
    {
        return Err("Recording has no playable duration".into());
    }
    Ok(())
}

pub(super) fn record(
    programs: &Programs,
    directory: &Path,
    stamp: &str,
    target: &Target,
    audio: Audio,
    stop: &AtomicBool,
    sender: &Sender<Event>,
) -> Result<Option<PathBuf>, String> {
    let selected = encoder(programs, audio, stop);
    if stop.load(Ordering::Acquire) {
        return Ok(None);
    }
    let encoder = selected?;
    let path = reserve(directory, stamp)?;
    if stop.load(Ordering::Acquire) {
        let _ = fs::remove_file(&path);
        return Ok(None);
    }
    let result = (|| {
        let mut process = Process::spawn(&mut command(programs, target, audio, encoder, &path))?;
        let _ = sender.send(Event::Started(Instant::now()));
        let mut deadline = None;
        loop {
            if process
                .child
                .try_wait()
                .map_err(|e| e.to_string())?
                .is_some()
            {
                break;
            }
            if stop.load(Ordering::Acquire) && deadline.is_none() {
                process.interrupt()?;
                deadline = Some(Instant::now() + STOP_TIMEOUT);
            }
            if deadline.is_some_and(|d| Instant::now() >= d) {
                return Err("Recorder did not finish within five seconds of Stop".into());
            }
            thread::sleep(POLL);
        }
        let (status, _, stderr) = process.output()?;
        if stop.load(Ordering::Acquire) && fs::metadata(&path).is_ok_and(|m| m.len() == 0) {
            let _ = fs::remove_file(&path);
            return Ok(None);
        }
        if !status.success() {
            return Err(format!("wf-recorder exited with {status}: {stderr}"));
        }
        validate(programs, &path, audio)?;
        if audio != Audio::Off
            && [
                "Recording won't have audio",
                "Failed to read from PulseAudio",
                "Failed to connect to PulseAudio",
            ]
            .iter()
            .any(|message| stderr.contains(message))
        {
            return Err(format!("The recorder reported an audio failure: {stderr}"));
        }
        if deadline.is_none() {
            return Err("Recorder stopped unexpectedly".into());
        }
        Ok(Some(path.clone()))
    })();
    match result {
        Ok(result) => Ok(result),
        Err(error) => {
            if fs::metadata(&path).is_ok_and(|m| m.len() == 0) {
                let _ = fs::remove_file(&path);
                Err(error)
            } else {
                Err(format!(
                    "{error}. Partial recording retained at {}",
                    path.display()
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests;
