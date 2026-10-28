//! Bounded grim acquisition and PNG/file workers. No shell command expansion.
use gio::prelude::*;
use gtk4::gdk_pixbuf::{Colorspace, InterpType, Pixbuf, PixbufLoader, prelude::*};
use std::{
    ffi::OsStr,
    io::Write,
    path::{Path, PathBuf},
};
use way_shell_core::capture::{CaptureLayout, MAX_IMAGE_BYTES, RgbaImage, image_len};

pub fn available() -> bool {
    glib::find_program_in_path("grim").is_some()
}

struct Captures(Vec<(gio::Subprocess, (u32, u32))>);
impl Drop for Captures {
    fn drop(&mut self) {
        for (process, _) in &self.0 {
            if process.identifier().is_some() {
                process.force_exit();
            }
        }
    }
}
pub async fn acquire(layout: &CaptureLayout) -> Result<Vec<RgbaImage>, String> {
    acquire_program(layout, OsStr::new("grim")).await
}
async fn acquire_program(
    layout: &CaptureLayout,
    program: &OsStr,
) -> Result<Vec<RgbaImage>, String> {
    let mut processes = Captures(Vec::new());
    // Every process starts before any preview surface is constructed or mapped.
    for output in &layout.outputs {
        let scale = output.grim_scale()?;
        let size = (
            (output.bounds.width * scale) as u32,
            (output.bounds.height * scale) as u32,
        );
        image_len(size.0, size.1)?;
        let scale = scale.to_string();
        let argv = [
            "grim",
            "-o",
            &output.connector,
            "-s",
            &scale,
            "-t",
            "png",
            "-",
        ];
        let mut args: Vec<_> = argv.iter().map(OsStr::new).collect();
        args[0] = program;
        processes.0.push((
            gio::Subprocess::newv(
                &args,
                gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_SILENCE,
            )
            .map_err(|e| format!("Could not start grim: {e}"))?,
            size,
        ));
    }
    let mut images = Vec::new();
    for ((process, expected), output) in processes.0.iter().zip(&layout.outputs) {
        let stream = process.stdout_pipe().ok_or("grim has no image stream")?;
        let mut png = Vec::new();
        loop {
            let bytes = stream
                .read_bytes_future(64 * 1024, glib::Priority::DEFAULT)
                .await
                .map_err(|e| e.to_string())?;
            if bytes.is_empty() {
                break;
            }
            if bytes.len() > MAX_IMAGE_BYTES - png.len() {
                return Err("grim output exceeds the image limit".into());
            }
            png.try_reserve(bytes.len())
                .map_err(|_| "Could not allocate grim image data")?;
            png.extend_from_slice(&bytes);
        }
        process
            .wait_check_future()
            .await
            .map_err(|e| format!("grim could not capture {}: {e}", output.connector))?;
        let expected = *expected;
        let native_size = output.native_size;
        images.push(
            gio::spawn_blocking(move || decode_png(&png, expected, native_size))
                .await
                .map_err(|_| "Screenshot decoder stopped")??,
        );
    }
    Ok(images)
}

pub fn decode_png(
    png: &[u8],
    expected: (u32, u32),
    native_size: (u32, u32),
) -> Result<RgbaImage, String> {
    if png.get(..8) != Some(b"\x89PNG\r\n\x1a\n") || png.get(12..16) != Some(b"IHDR") {
        return Err("grim returned invalid PNG data".into());
    }
    let width = u32::from_be_bytes(png.get(16..20).ok_or("Truncated PNG")?.try_into().unwrap());
    let height = u32::from_be_bytes(png.get(20..24).ok_or("Truncated PNG")?.try_into().unwrap());
    image_len(width, height)?;
    if (width, height) != expected {
        return Err(format!(
            "grim image {width}×{height} does not match requested capture {}×{}",
            expected.0, expected.1
        ));
    }
    image_len(native_size.0, native_size.1)?;
    let loader = PixbufLoader::with_type("png").map_err(|e| e.to_string())?;
    loader.write(png).map_err(|e| e.to_string())?;
    loader.close().map_err(|e| e.to_string())?;
    let image = loader.pixbuf().ok_or("PNG has no pixels")?;
    if image.width() != width as i32
        || image.height() != height as i32
        || image.bits_per_sample() != 8
    {
        return Err("Invalid decoded screenshot dimensions".into());
    }
    let image = if expected == native_size {
        image
    } else {
        image
            .scale_simple(
                native_size.0 as i32,
                native_size.1 as i32,
                InterpType::Bilinear,
            )
            .ok_or("Could not resize screenshot to native dimensions")?
    };
    let (width, height) = native_size;
    let source = image.read_pixel_bytes();
    let channels = image.n_channels() as usize;
    let stride = image.rowstride() as usize;
    if !matches!(channels, 3 | 4) {
        return Err("Unsupported screenshot pixel format".into());
    }
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(image_len(width, height)?)
        .map_err(|_| "Could not allocate screenshot pixels")?;
    for y in 0..height as usize {
        for x in 0..width as usize {
            let start = y * stride + x * channels;
            let pixel = source
                .get(start..start + channels)
                .ok_or("Incomplete PNG pixels")?;
            pixels.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
        }
    }
    RgbaImage::new(width, height, pixels)
}
pub fn pixbuf(image: &RgbaImage) -> Pixbuf {
    Pixbuf::from_bytes(
        &glib::Bytes::from_owned(image.pixels.clone()),
        Colorspace::Rgb,
        true,
        8,
        image.width as i32,
        image.height as i32,
        image.width as i32 * 4,
    )
}
pub fn encode_png(image: &RgbaImage) -> Result<Vec<u8>, String> {
    pixbuf(image)
        .save_to_bufferv("png", &[])
        .map_err(|e| e.to_string())
}

pub fn destination(value: &str, pictures: Option<&Path>, home: &Path) -> Result<PathBuf, String> {
    let path = if value.is_empty() {
        pictures.unwrap_or(home).join("Screenshots")
    } else if let Some(rest) = value.strip_prefix("~/") {
        home.join(rest)
    } else {
        PathBuf::from(value)
    };
    if !path.is_absolute() {
        return Err("Screenshot directory must be absolute or start with ~/".into());
    }
    Ok(path)
}
/// create_new provides collision safety even across simultaneous writers.
pub fn save_png(directory: &Path, stamp: &str, png: &[u8]) -> Result<PathBuf, String> {
    std::fs::create_dir_all(directory)
        .map_err(|e| format!("Could not create {}: {e}", directory.display()))?;
    for suffix in 0..10_000 {
        let name = if suffix == 0 {
            format!("Screenshot-{stamp}.png")
        } else {
            format!("Screenshot-{stamp}-{suffix}.png")
        };
        let path = directory.join(name);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                if let Err(error) = file.write_all(png).and_then(|()| file.sync_all()) {
                    drop(file);
                    let _ = std::fs::remove_file(&path);
                    return Err(format!("Could not save {}: {error}", path.display()));
                }
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("Could not save {}: {error}", path.display())),
        }
    }
    Err("Too many screenshot filename collisions".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn directories_collisions_write_errors_and_png_validation() {
        let home = Path::new("/home/example");
        assert_eq!(
            destination("", None, home).unwrap(),
            home.join("Screenshots")
        );
        assert_eq!(
            destination("", Some(Path::new("/pictures")), home).unwrap(),
            PathBuf::from("/pictures/Screenshots")
        );
        assert_eq!(
            destination("~/shots", None, home).unwrap(),
            home.join("shots")
        );
        assert_eq!(
            destination("/custom", None, home).unwrap(),
            PathBuf::from("/custom")
        );
        for invalid in ["relative", "~", "$HOME/shots"] {
            assert!(destination(invalid, None, home).is_err());
        }
        let dir =
            std::env::temp_dir().join(format!("way-shell-screenshot-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let image = RgbaImage::new(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 255]).unwrap();
        let png = encode_png(&image).unwrap();
        assert_eq!(decode_png(&png, (2, 1), (2, 1)).unwrap(), image);
        assert!(decode_png(&png[..png.len() / 2], (2, 1), (2, 1)).is_err());
        assert!(decode_png(b"not a PNG", (2, 1), (2, 1)).is_err());
        assert!(decode_png(&png, (1, 2), (2, 1)).is_err());
        let first = save_png(&dir, "same", &png).unwrap();
        let second = save_png(&dir, "same", b"other").unwrap();
        assert_ne!(first, second);
        assert_eq!(std::fs::read(&first).unwrap(), png);
        assert!(save_png(&first, "bad", &png).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod acquisition_tests {
    use super::*;
    use std::{os::unix::fs::PermissionsExt, time::Duration};
    use way_shell_core::capture::{CaptureOutput, Rect};
    #[test]
    fn subprocess_rounding_failure_corruption_timeout_and_cancellation_cleanup() {
        let directory =
            std::env::temp_dir().join(format!("way-shell-grim-test-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let program = directory.join("grim");
        let layout = CaptureLayout {
            outputs: vec![CaptureOutput {
                connector: "test".into(),
                bounds: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 2.0,
                    height: 1.0,
                },
                scale: 1.0,
                transform: "normal".into(),
                native_size: (2, 1),
            }],
            windows: vec![],
            focused_output: None,
        };
        let script = |body: &str| {
            std::fs::write(&program, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        };
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| {
                script("exit 1");
                assert!(
                    context
                        .block_on(acquire_program(&layout, program.as_os_str()))
                        .is_err()
                );
                script("printf invalid");
                assert!(
                    context
                        .block_on(acquire_program(&layout, program.as_os_str()))
                        .is_err()
                );
                let native = RgbaImage::new(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 255]).unwrap();
                let rounded = RgbaImage::new(
                    8,
                    6,
                    (0..48)
                        .flat_map(|i| {
                            if i % 8 < 4 {
                                [255, 0, 0, 255]
                            } else {
                                [0, 0, 255, 255]
                            }
                        })
                        .collect(),
                )
                .unwrap();
                let native_path = directory.join("native.png");
                let rounded_path = directory.join("rounded.png");
                std::fs::write(&native_path, encode_png(&native).unwrap()).unwrap();
                std::fs::write(&rounded_path, encode_png(&rounded).unwrap()).unwrap();
                script(&format!(
                    "if [ \"$2\" = test ]; then cat '{}'; else cat '{}'; fi",
                    native_path.display(),
                    rounded_path.display()
                ));
                let fractional_layout = CaptureLayout {
                    outputs: vec![
                        layout.outputs[0].clone(),
                        CaptureOutput {
                            connector: "test-fractional".into(),
                            bounds: Rect {
                                x: 2.0,
                                y: 0.0,
                                width: 4.0,
                                height: 3.0,
                            },
                            scale: 1.75,
                            native_size: (7, 6),
                            ..layout.outputs[0].clone()
                        },
                    ],
                    ..layout.clone()
                }
                .validate()
                .unwrap();
                let images = context
                    .block_on(acquire_program(&fractional_layout, program.as_os_str()))
                    .unwrap();
                assert_eq!(images[0], native);
                assert_eq!((images[1].width, images[1].height), (7, 6));
                for row in images[1].pixels.as_chunks::<28>().0 {
                    assert_eq!(&row[..4], &[255, 0, 0, 255]);
                    assert_eq!(&row[6 * 4..], &[0, 0, 255, 255]);
                }
                let pid_path = directory.join("pid");
                script(&format!(
                    "echo $$ > '{}'; exec sleep 30",
                    pid_path.display()
                ));
                assert!(
                    context
                        .block_on(glib::future_with_timeout(
                            Duration::from_millis(100),
                            acquire_program(&layout, program.as_os_str())
                        ))
                        .is_err()
                );
                let pid = std::fs::read_to_string(pid_path).unwrap();
                let deadline = std::time::Instant::now() + Duration::from_secs(2);
                while Path::new(&format!("/proc/{}", pid.trim())).exists() {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "Canceled grim process leaked"
                    );
                    context.block_on(glib::timeout_future(Duration::from_millis(10)));
                }
            })
            .unwrap();
        std::fs::remove_dir_all(directory).unwrap();
    }
}
