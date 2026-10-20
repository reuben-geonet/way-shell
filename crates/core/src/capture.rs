//! Frozen desktop geometry and pixel composition, independent of GTK.
use serde::Deserialize;
use std::sync::Arc;

/// Bound both decoded inputs and composed outputs to 512 MiB of RGBA pixels.
pub const MAX_IMAGE_BYTES: usize = 512 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
impl Rect {
    pub fn valid(self) -> bool {
        [self.x, self.y, self.width, self.height]
            .iter()
            .all(|v| v.is_finite() && v.abs() < 1e9)
            && self.width > 0.0
            && self.height > 0.0
    }
    pub fn right(self) -> f64 {
        self.x + self.width
    }
    pub fn bottom(self) -> f64 {
        self.y + self.height
    }
    pub fn contains(self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }
    pub fn intersection(self, other: Self) -> Option<Self> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let rect = Self {
            x,
            y,
            width: self.right().min(other.right()) - x,
            height: self.bottom().min(other.bottom()) - y,
        };
        rect.valid().then_some(rect)
    }
    pub fn union(self, other: Self) -> Self {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Self {
            x,
            y,
            width: self.right().max(other.right()) - x,
            height: self.bottom().max(other.bottom()) - y,
        }
    }
    pub fn from_points(x: f64, y: f64, end_x: f64, end_y: f64) -> Self {
        Self {
            x: x.min(end_x),
            y: y.min(end_y),
            width: (end_x - x).abs().max(1.0),
            height: (end_y - y).abs().max(1.0),
        }
    }
    pub fn clamped(self, bounds: Self) -> Self {
        let width = self.width.clamp(1.0, bounds.width);
        let height = self.height.clamp(1.0, bounds.height);
        Self {
            x: self.x.clamp(bounds.x, bounds.right() - width),
            y: self.y.clamp(bounds.y, bounds.bottom() - height),
            width,
            height,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CaptureOutput {
    pub connector: String,
    pub bounds: Rect,
    pub scale: f64,
    /// The compositor's transform string. Grim already applies this transform.
    pub transform: String,
    /// Oriented native size (width and height exchanged for quarter turns).
    pub native_size: (u32, u32),
}
impl CaptureOutput {
    /// Grim truncates logical size × requested scale. Fractional output sizes
    /// can otherwise lose a native row or column. Prefer a scale producing the
    /// exact native dimensions on both axes. If their rounding intervals do not
    /// overlap, cover both axes and resize the captured image to native size.
    pub fn grim_scale(&self) -> Result<f64, String> {
        let (width, height) = self.native_size;
        let lower = (width as f64 / self.bounds.width).max(height as f64 / self.bounds.height);
        let upper = ((width as f64 + 1.0) / self.bounds.width)
            .min((height as f64 + 1.0) / self.bounds.height);
        let scale = if lower < upper {
            lower + (upper - lower) / 2.0
        } else {
            // Stay above the boundary so floating-point rounding cannot truncate
            // an axis below its native size.
            lower.next_up()
        };
        if !self.bounds.valid() || !scale.is_finite() || scale <= 0.0 {
            return Err(format!(
                "Cannot capture native pixels for {} with this logical geometry",
                self.connector
            ));
        }
        Ok(scale)
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct CaptureWindow {
    pub id: u64,
    pub title: String,
    pub bounds: Rect,
    pub focused: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct CaptureLayout {
    pub outputs: Vec<CaptureOutput>,
    pub windows: Vec<CaptureWindow>,
    pub focused_output: Option<String>,
}
impl CaptureLayout {
    pub fn validate(mut self) -> Result<Self, String> {
        self.outputs.sort_by(|a, b| a.connector.cmp(&b.connector));
        if self.outputs.is_empty() {
            return Err("No active displays are available".into());
        }
        let mut total = 0usize;
        for (i, output) in self.outputs.iter().enumerate() {
            if output.connector.is_empty()
                || output.connector.contains('\0')
                || !output.bounds.valid()
                || !output.scale.is_finite()
                || output.scale <= 0.0
                || output.scale > 16.0
                || (i > 0 && self.outputs[i - 1].connector == output.connector)
            {
                return Err("Compositor returned an invalid display layout".into());
            }
            total = total
                .checked_add(image_len(output.native_size.0, output.native_size.1)?)
                .ok_or("Display images exceed the memory limit")?;
            if total > MAX_IMAGE_BYTES {
                return Err("Display images exceed the memory limit".into());
            }
        }
        let bounds = self.bounds();
        self.windows.retain_mut(|window| {
            if !window.bounds.valid() {
                return false;
            }
            let Some(clipped) = window.bounds.intersection(bounds) else {
                return false;
            };
            if !self
                .outputs
                .iter()
                .any(|output| window.bounds.intersection(output.bounds).is_some())
            {
                return false;
            }
            window.bounds = clipped;
            true
        });
        Ok(self)
    }
    pub fn bounds(&self) -> Rect {
        self.outputs
            .iter()
            .map(|o| o.bounds)
            .reduce(Rect::union)
            .unwrap_or_default()
    }
    pub fn launching_output(&self, connector: Option<&str>) -> usize {
        connector
            .or(self.focused_output.as_deref())
            .and_then(|name| self.outputs.iter().position(|o| o.connector == name))
            .unwrap_or(0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Area,
    Window,
    Screen,
    All,
}
#[derive(Clone, Debug)]
pub struct Selection {
    pub mode: Mode,
    pub area: Rect,
    pub window: usize,
    pub output: usize,
}
impl Selection {
    pub fn new(layout: &CaptureLayout, launching: usize) -> Self {
        let b = layout.outputs[launching].bounds;
        Self {
            mode: Mode::Area,
            area: Rect {
                x: b.x + b.width / 4.0,
                y: b.y + b.height / 4.0,
                width: b.width / 2.0,
                height: b.height / 2.0,
            },
            window: layout.windows.iter().position(|w| w.focused).unwrap_or(0),
            output: launching,
        }
    }
    pub fn bounds(&self, layout: &CaptureLayout) -> Option<Rect> {
        match self.mode {
            Mode::Area => Some(self.area),
            Mode::Window => layout.windows.get(self.window).map(|w| w.bounds),
            Mode::Screen => layout.outputs.get(self.output).map(|o| o.bounds),
            Mode::All => Some(layout.bounds()),
        }
    }
    pub fn click(&mut self, layout: &CaptureLayout, x: f64, y: f64) {
        match self.mode {
            Mode::Screen => {
                if let Some(i) = layout.outputs.iter().position(|o| o.bounds.contains(x, y)) {
                    self.output = i;
                }
            }
            Mode::Window => {
                let hits: Vec<_> = layout
                    .windows
                    .iter()
                    .enumerate()
                    .filter(|(_, w)| w.bounds.contains(x, y))
                    .map(|(i, _)| i)
                    .collect();
                if !hits.is_empty() {
                    self.window = hits
                        .iter()
                        .position(|i| *i == self.window)
                        .map(|i| hits[(i + 1) % hits.len()])
                        .unwrap_or(hits[0]);
                }
            }
            _ => (),
        }
    }
    pub fn adjust(&mut self, layout: &CaptureLayout, dx: f64, dy: f64, resize: bool) {
        match self.mode {
            Mode::Area => {
                if resize {
                    self.area.width = (self.area.width + dx).max(1.0);
                    self.area.height = (self.area.height + dy).max(1.0);
                } else {
                    self.area.x += dx;
                    self.area.y += dy;
                }
                self.area = self.area.clamped(layout.bounds());
            }
            Mode::Window | Mode::Screen => {
                let (index, count) = if self.mode == Mode::Window {
                    (&mut self.window, layout.windows.len())
                } else {
                    (&mut self.output, layout.outputs.len())
                };
                if count > 0 {
                    *index = (*index as isize + if dx + dy > 0.0 { 1 } else { -1 })
                        .rem_euclid(count as isize) as usize;
                }
            }
            Mode::All => (),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<[u8]>,
}
pub fn image_len(width: u32, height: u32) -> Result<usize, String> {
    let size = (width as usize)
        .checked_mul(height as usize)
        .and_then(|v| v.checked_mul(4))
        .filter(|n| *n > 0 && *n <= MAX_IMAGE_BYTES)
        .ok_or("Image dimensions exceed the memory limit")?;
    Ok(size)
}
impl RgbaImage {
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Result<Self, String> {
        if pixels.len() != image_len(width, height)? {
            return Err("Incomplete screenshot image".into());
        }
        Ok(Self {
            width,
            height,
            pixels: pixels.into(),
        })
    }
}
/// All edges use this same mapping so adjacent outputs cannot develop seams.
fn edge(value: f64, origin: f64, scale: f64) -> u32 {
    ((value - origin) * scale).round().max(0.0) as u32
}
pub fn dimensions(
    layout: &CaptureLayout,
    selection: &Selection,
) -> Result<(u32, u32, f64), String> {
    if selection.mode == Mode::Screen {
        let o = layout
            .outputs
            .get(selection.output)
            .ok_or("No selected display")?;
        return Ok((o.native_size.0, o.native_size.1, o.scale));
    }
    let bounds = selection
        .bounds(layout)
        .filter(|b| b.valid())
        .ok_or("No selected region")?;
    let scale = layout
        .outputs
        .iter()
        .filter(|o| o.bounds.intersection(bounds).is_some())
        .map(|o| {
            o.scale
                .max(o.native_size.0 as f64 / o.bounds.width)
                .max(o.native_size.1 as f64 / o.bounds.height)
        })
        .reduce(f64::max)
        .ok_or("Selection does not intersect a display")?;
    let width = edge(bounds.right(), bounds.x, scale);
    let height = edge(bounds.bottom(), bounds.y, scale);
    image_len(width, height)?;
    Ok((width, height, scale))
}
pub fn compose(
    layout: &CaptureLayout,
    images: &[RgbaImage],
    selection: &Selection,
) -> Result<RgbaImage, String> {
    if images.len() != layout.outputs.len() {
        return Err("Incomplete display images".into());
    }
    for (o, i) in layout.outputs.iter().zip(images) {
        if (i.width, i.height) != o.native_size || i.pixels.len() != image_len(i.width, i.height)? {
            return Err("Screenshot dimensions do not match the display".into());
        }
    }
    if selection.mode == Mode::Screen {
        return images
            .get(selection.output)
            .cloned()
            .ok_or("No selected display".into());
    }
    let bounds = selection.bounds(layout).ok_or("No selected region")?;
    let (width, height, scale) = dimensions(layout, selection)?;
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(image_len(width, height)?)
        .map_err(|_| "Could not allocate screenshot image")?;
    pixels.resize(image_len(width, height)?, 0);
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel[3] = 255;
    }
    for (output, image) in layout.outputs.iter().zip(images) {
        let Some(crop) = bounds.intersection(output.bounds) else {
            continue;
        };
        let x0 = edge(crop.x, bounds.x, scale).min(width);
        let y0 = edge(crop.y, bounds.y, scale).min(height);
        let x1 = edge(crop.right(), bounds.x, scale).min(width);
        let y1 = edge(crop.bottom(), bounds.y, scale).min(height);
        for y in y0..y1 {
            let gy = bounds.y + (y as f64 + 0.5) / scale;
            let sy = ((gy - output.bounds.y) / output.bounds.height * image.height as f64)
                .floor()
                .clamp(0.0, (image.height - 1) as f64) as usize;
            for x in x0..x1 {
                let gx = bounds.x + (x as f64 + 0.5) / scale;
                let sx = ((gx - output.bounds.x) / output.bounds.width * image.width as f64)
                    .floor()
                    .clamp(0.0, (image.width - 1) as f64) as usize;
                let source = (sy * image.width as usize + sx) * 4;
                let target = (y as usize * width as usize + x as usize) * 4;
                pixels[target..target + 4].copy_from_slice(&image.pixels[source..source + 4]);
                pixels[target + 3] = 255;
            }
        }
    }
    RgbaImage::new(width, height, pixels)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn output(name: &str, bounds: Rect, scale: f64) -> CaptureOutput {
        CaptureOutput {
            connector: name.into(),
            bounds,
            scale,
            transform: "normal".into(),
            native_size: (
                (bounds.width * scale).round() as u32,
                (bounds.height * scale).round() as u32,
            ),
        }
    }
    fn image(w: u32, h: u32, c: u8) -> RgbaImage {
        RgbaImage::new(w, h, [c, 0, 0, 255].repeat((w * h) as usize)).unwrap()
    }
    #[test]
    fn grim_fractional_rounding_preserves_odd_native_dimensions() {
        let output = CaptureOutput {
            connector: "portrait".into(),
            bounds: Rect {
                x: 0.0,
                y: 0.0,
                width: 596.0,
                height: 771.0,
            },
            scale: 1.5,
            transform: "90".into(),
            native_size: (894, 1157),
        };
        let scale = output.grim_scale().unwrap();
        assert_eq!(
            (
                (output.bounds.width * scale) as u32,
                (output.bounds.height * scale) as u32
            ),
            output.native_size
        );
    }
    #[test]
    fn grim_fractional_rounding_without_a_common_scale_covers_both_axes() {
        for (width, height, native_size, captured_size, transform) in [
            (1462.0, 822.0, (2560, 1440), (2561, 1440), "normal"),
            (822.0, 1462.0, (1440, 2560), (1440, 2561), "90"),
        ] {
            let output = CaptureOutput {
                connector: "fractional".into(),
                bounds: Rect {
                    x: 0.0,
                    y: 0.0,
                    width,
                    height,
                },
                scale: 1.75,
                transform: transform.into(),
                native_size,
            };
            let scale = output.grim_scale().unwrap();
            assert_eq!(
                ((width * scale) as u32, (height * scale) as u32),
                captured_size
            );
        }
    }
    #[test]
    fn negative_stacked_portrait_fractional_and_black_gaps() {
        let layout = CaptureLayout {
            outputs: vec![
                output(
                    "A",
                    Rect {
                        x: -2.0,
                        y: -2.0,
                        width: 2.0,
                        height: 2.0,
                    },
                    1.5,
                ),
                output(
                    "B",
                    Rect {
                        x: 0.0,
                        y: 1.0,
                        width: 1.0,
                        height: 3.0,
                    },
                    1.0,
                ),
            ],
            windows: vec![],
            focused_output: None,
        }
        .validate()
        .unwrap();
        let mut selection = Selection::new(&layout, 0);
        selection.mode = Mode::All;
        let result = compose(&layout, &[image(3, 3, 10), image(1, 3, 20)], &selection).unwrap();
        assert_eq!((result.width, result.height), (5, 9));
        assert_eq!(&result.pixels[..4], &[10, 0, 0, 255]);
        assert_eq!(&result.pixels[4 * 4..5 * 4], &[0, 0, 0, 255]);
        selection.mode = Mode::Screen;
        selection.output = 1;
        assert_eq!(
            compose(&layout, &[image(3, 3, 10), image(1, 3, 20)], &selection).unwrap(),
            image(1, 3, 20)
        );
    }
    #[test]
    fn shared_edges_and_crop_orientation() {
        let layout = CaptureLayout {
            outputs: vec![
                output(
                    "A",
                    Rect {
                        x: -2.0,
                        y: 0.0,
                        width: 2.0,
                        height: 2.0,
                    },
                    1.25,
                ),
                output(
                    "B",
                    Rect {
                        x: 0.0,
                        y: 0.0,
                        width: 2.0,
                        height: 2.0,
                    },
                    1.5,
                ),
            ],
            windows: vec![],
            focused_output: Some("B".into()),
        }
        .validate()
        .unwrap();
        let images = [
            image(3, 3, 10),
            RgbaImage::new(3, 3, (0..9).flat_map(|v| [v, 1, 2, 255]).collect()).unwrap(),
        ];
        let mut selection = Selection::new(&layout, layout.launching_output(None));
        selection.mode = Mode::All;
        let result = compose(&layout, &images, &selection).unwrap();
        assert_eq!((result.width, result.height), (6, 3));
        assert!(
            result
                .pixels
                .as_chunks::<4>()
                .0
                .iter()
                .all(|p| p[0] != 0 || p[1] != 0)
        );
        selection.mode = Mode::Area;
        selection.area = Rect {
            x: 0.0,
            y: 0.0,
            width: 2.0,
            height: 2.0,
        };
        assert_eq!(compose(&layout, &images, &selection).unwrap(), images[1]);
        assert_eq!(layout.launching_output(None), 1);
    }
    #[test]
    fn invalid_dimensions_clipping_overlap_cycle_and_navigation() {
        assert!(image_len(u32::MAX, u32::MAX).is_err());
        assert!(RgbaImage::new(2, 2, vec![0; 15]).is_err());
        let bounds = Rect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
        };
        let window = CaptureWindow {
            id: 1,
            title: "test".into(),
            bounds: Rect {
                x: -10.0,
                y: 0.0,
                width: 50.0,
                height: 50.0,
            },
            focused: true,
        };
        let layout = CaptureLayout {
            outputs: vec![output("A", bounds, 1.0)],
            windows: vec![window.clone(), CaptureWindow { id: 2, ..window }],
            focused_output: None,
        }
        .validate()
        .unwrap();
        assert_eq!(layout.windows[0].bounds.width, 40.0);
        let mut s = Selection::new(&layout, 0);
        s.mode = Mode::Window;
        s.click(&layout, 5.0, 5.0);
        assert_eq!(s.window, 1);
        s.click(&layout, 5.0, 5.0);
        assert_eq!(s.window, 0);
        s.adjust(&layout, -1.0, 0.0, false);
        assert_eq!(s.window, 1);
        s.mode = Mode::Area;
        s.adjust(&layout, 100.0, 100.0, false);
        assert_eq!(s.area.right(), 100.0);
    }
}
