//! A soft spotlight that blooms from the island and fades as it spreads.
//! One bounded, click-through bitmap exists only during the effect.
use lumo_core::geom::Color;
use lumo_core::settings::FocusLightColor;
use windows::core::{w, Result};
use windows::Win32::Foundation::{COLORREF, HWND, POINT, RECT, SIZE};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const DURATION: f64 = 3.0;

pub fn color(choice: FocusLightColor, accent: Color) -> Color {
    match choice {
        FocusLightColor::WindowsAccent => accent,
        FocusLightColor::Blue => Color::hex(0x64ACFF),
        FocusLightColor::Mint => Color::hex(0x67E8BE),
        FocusLightColor::Rose => Color::hex(0xFF88B7),
        FocusLightColor::Gold => Color::hex(0xFFD080),
        FocusLightColor::Violet => Color::hex(0xBA9AFF),
    }
}

struct Layer {
    hwnd: HWND,
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    bits: *mut u8,
    rect: RECT,
    visible: bool,
}

impl Layer {
    fn new(owner: HWND, rect: RECT) -> Result<Self> {
        let mut layer = Self {
            hwnd: HWND::default(),
            dc: HDC::default(),
            bitmap: HBITMAP::default(),
            previous: HGDIOBJ::default(),
            bits: std::ptr::null_mut(),
            rect,
            visible: false,
        };
        unsafe {
            layer.hwnd = CreateWindowExW(
                WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_TOPMOST
                    | WS_EX_TOOLWINDOW
                    | WS_EX_NOACTIVATE,
                crate::window::VISUAL_CLASS,
                w!("Lumo focus light"),
                WS_POPUP,
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                owner,
                None,
                GetModuleHandleW(None)?,
                None,
            )?;
            layer.dc = CreateCompatibleDC(None);
            if layer.dc.is_invalid() {
                return Err(windows::core::Error::from_win32());
            }
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: rect.right - rect.left,
                    biHeight: -(rect.bottom - rect.top),
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            layer.bitmap = CreateDIBSection(layer.dc, &info, DIB_RGB_COLORS, &mut bits, None, 0)?;
            layer.bits = bits.cast();
            layer.previous = SelectObject(layer.dc, layer.bitmap);
            if layer.bits.is_null() || layer.previous.is_invalid() {
                return Err(windows::core::Error::from_win32());
            }
        }
        Ok(layer)
    }

    fn show(&mut self, alpha: u8) -> Result<()> {
        if alpha == 0 {
            if self.visible {
                unsafe {
                    let _ = ShowWindow(self.hwnd, SW_HIDE);
                }
            }
            self.visible = false;
            return Ok(());
        }
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: alpha,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let position = POINT {
            x: self.rect.left,
            y: self.rect.top,
        };
        let size = SIZE {
            cx: self.rect.right - self.rect.left,
            cy: self.rect.bottom - self.rect.top,
        };
        let source = POINT::default();
        let info = UPDATELAYEREDWINDOWINFO {
            cbSize: std::mem::size_of::<UPDATELAYEREDWINDOWINFO>() as u32,
            hdcDst: HDC::default(),
            pptDst: &position,
            psize: &size,
            hdcSrc: self.dc,
            pptSrc: &source,
            crKey: COLORREF(0),
            pblend: &blend,
            dwFlags: ULW_ALPHA,
            prcDirty: std::ptr::null(),
        };
        unsafe {
            if !UpdateLayeredWindowIndirect(self.hwnd, &info).as_bool() {
                return Err(windows::core::Error::from_win32());
            }
            if !self.visible {
                let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            }
        }
        self.visible = true;
        Ok(())
    }
}

impl Drop for Layer {
    fn drop(&mut self) {
        unsafe {
            if !self.previous.is_invalid() {
                SelectObject(self.dc, self.previous);
            }
            if !self.bitmap.is_invalid() {
                let _ = DeleteObject(self.bitmap);
            }
            if !self.dc.is_invalid() {
                let _ = DeleteDC(self.dc);
            }
            if !self.hwnd.is_invalid() {
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }
}

/// A small lookup table supplies a soft falloff with a transparent boundary.
/// Axis weights make the bloom separable, so each pixel needs one multiply
/// and one premultiplied-color lookup, without blur or per-pixel exponentials.
fn falloff() -> [u8; 257] {
    let edge = (-4.5_f32).exp();
    std::array::from_fn(|i| {
        let distance = i as f32 / 256.0;
        (((-4.5 * distance * distance).exp() - edge) / (1.0 - edge) * 255.0).max(0.0) as u8
    })
}

fn envelope(elapsed: f64) -> (f32, u8) {
    let progress = (elapsed / DURATION).clamp(0.0, 1.0) as f32;
    let spread = 1.0 - (1.0 - progress).powi(3);
    let attack = (elapsed / 0.18).clamp(0.0, 1.0) as f32;
    let opacity = (attack * (1.0 - progress).powf(0.85) * 115.0) as u8;
    (spread, opacity)
}

pub struct Spotlight {
    layer: Layer,
    colors: [u32; 256],
    falloff: [u8; 257],
    x_distance: Vec<f32>,
    y_distance: Vec<f32>,
    x_weight: Vec<u8>,
    y_weight: Vec<u8>,
    initial_radius: (f32, f32),
    final_radius: (f32, f32),
    started: f64,
    last_frame: Option<f64>,
    opacity: u8,
}

impl Spotlight {
    pub fn new(
        owner: HWND,
        monitor: RECT,
        island: RECT,
        scale: f32,
        color: Color,
        now: f64,
    ) -> Result<Self> {
        let scale = scale.max(0.5);
        let origin_x = ((island.left + island.right) / 2).clamp(monitor.left, monitor.right - 1);
        let source_offset = ((island.bottom - island.top) as f32 * 0.25).min(12.0 * scale) as i32;
        let origin_y = (island.top + source_offset).clamp(monitor.top, monitor.bottom - 1);
        let screen_height = (monitor.bottom - monitor.top) as f32;
        // The reference is a shallow bloom from the top center. Bound its
        // depth to avoid allocating or repainting a full-monitor surface.
        let depth = (screen_height * 0.32).min(220.0 * scale).max(1.0);
        let rect = RECT {
            left: monitor.left,
            top: monitor.top.max(origin_y - depth.ceil() as i32),
            right: monitor.right,
            bottom: monitor.bottom.min(origin_y + depth.ceil() as i32),
        };
        let width = (rect.right - rect.left) as usize;
        let height = (rect.bottom - rect.top) as usize;
        let red = (color.r * 255.0) as u16;
        let green = (color.g * 255.0) as u16;
        let blue = (color.b * 255.0) as u16;
        let colors = std::array::from_fn(|alpha| {
            let alpha = alpha as u16;
            u32::from_le_bytes([
                (blue * alpha / 255) as u8,
                (green * alpha / 255) as u8,
                (red * alpha / 255) as u8,
                alpha as u8,
            ])
        });
        let initial_radius = (
            ((island.right - island.left) as f32 * 0.35).clamp(28.0 * scale, 100.0 * scale),
            ((island.bottom - island.top) as f32 * 0.5).clamp(14.0 * scale, 28.0 * scale),
        );
        let final_radius = (
            (origin_x - monitor.left).max(monitor.right - origin_x) as f32 * 1.8 + 8.0 * scale,
            depth,
        );
        Ok(Self {
            layer: Layer::new(owner, rect)?,
            colors,
            falloff: falloff(),
            x_distance: (0..width)
                .map(|x| (rect.left as f32 + x as f32 + 0.5 - origin_x as f32).abs())
                .collect(),
            y_distance: (0..height)
                .map(|y| (rect.top as f32 + y as f32 + 0.5 - origin_y as f32).abs())
                .collect(),
            x_weight: vec![0; width],
            y_weight: vec![0; height],
            initial_radius,
            final_radius,
            started: now,
            last_frame: None,
            opacity: 0,
        })
    }

    /// The existing clock runs this three-second effect near 30 Hz. All
    /// axis tables and the bitmap are reused, with no per-frame allocations.
    pub fn frame(&mut self, now: f64) -> Result<bool> {
        if now >= self.started + DURATION {
            return Ok(false);
        }
        if self
            .last_frame
            .is_some_and(|last| now - last < 1.0 / 30.0 - 0.001)
        {
            return Ok(true);
        }
        self.last_frame = Some(now);
        let (spread, opacity) = envelope((now - self.started).max(0.0));
        self.opacity = opacity;
        if opacity == 0 {
            self.layer.show(0)?;
            return Ok(true);
        }
        let rx = self.initial_radius.0 + (self.final_radius.0 - self.initial_radius.0) * spread;
        let ry = self.initial_radius.1 + (self.final_radius.1 - self.initial_radius.1) * spread;
        for (weight, distance) in self.x_weight.iter_mut().zip(&self.x_distance) {
            *weight = self.falloff[(distance / rx * 256.0).min(256.0) as usize];
        }
        for (weight, distance) in self.y_weight.iter_mut().zip(&self.y_distance) {
            *weight = self.falloff[(distance / ry * 256.0).min(256.0) as usize];
        }
        let width = self.x_weight.len();
        let length = width * self.y_weight.len();
        // DIB storage is four-byte aligned and owns exactly these pixels.
        let pixels =
            unsafe { std::slice::from_raw_parts_mut(self.layer.bits.cast::<u32>(), length) };
        for (row, &y) in pixels.chunks_exact_mut(width).zip(&self.y_weight) {
            if y == 0 {
                row.fill(0);
                continue;
            }
            for (pixel, &x) in row.iter_mut().zip(&self.x_weight) {
                *pixel = self.colors[(u16::from(x) * u16::from(y) / 255) as usize];
            }
        }
        self.layer.show(opacity)?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bloom_expands_smoothly_and_is_transparent_at_the_end() {
        let kernel = falloff();
        assert_eq!(kernel[0], 255);
        assert_eq!(kernel[256], 0);
        assert!(kernel.windows(2).all(|pair| pair[0] >= pair[1]));
        assert_eq!(envelope(0.0), (0.0, 0));
        let first = envelope(0.3);
        let middle = envelope(1.2);
        let late = envelope(2.7);
        assert!(first.0 < middle.0 && middle.0 < late.0);
        assert!(first.1 > middle.1 && middle.1 > late.1);
        assert_eq!(envelope(DURATION), (1.0, 0));
    }

    #[test]
    #[ignore = "native spotlight is placed off screen; checks input, focus and cleanup"]
    fn spotlight_preserves_focus_passes_input_and_releases_its_window() {
        use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
        unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        crate::window::register_classes(proc, proc).unwrap();
        let owner = crate::window::create_island_window().unwrap();
        let monitor = RECT {
            left: -4200,
            top: -1800,
            right: -3400,
            bottom: -1200,
        };
        let island = RECT {
            left: -3920,
            top: -1792,
            right: -3680,
            bottom: -1752,
        };
        let foreground = unsafe { GetForegroundWindow() };
        let mut light =
            Spotlight::new(owner, monitor, island, 1.0, Color::hex(0x67E8BE), 100.0).unwrap();
        let hwnd = light.layer.hwnd;
        unsafe {
            let style = WINDOW_EX_STYLE(GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32);
            assert!(style.contains(WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW));
            assert_eq!(
                SendMessageW(hwnd, WM_NCHITTEST, WPARAM(0), LPARAM(0)).0,
                HTTRANSPARENT as isize
            );
        }
        let width = (light.layer.rect.right - light.layer.rect.left) as usize;
        let height = (light.layer.rect.bottom - light.layer.rect.top) as usize;
        assert!(
            height < 240 && width == 800,
            "only the upper bloom area has bitmap storage"
        );
        for (index, elapsed) in [0.1, 0.4, 0.9, 1.7, 2.7].into_iter().enumerate() {
            assert!(light.frame(100.0 + elapsed).unwrap());
            assert_eq!(unsafe { GetForegroundWindow() }, foreground);
            assert!(
                light.x_weight[width / 2] >= light.x_weight[0],
                "brightness spreads outward from the island"
            );
            if let Some(root) = std::env::var_os("LUMO_FOCUS_QA") {
                // A fixture background under only this effect's own DIB.
                // No desktop or personal application pixels are captured.
                let mut rgba = vec![0; 800 * 600 * 4];
                for pixel in rgba.chunks_exact_mut(4) {
                    pixel.copy_from_slice(&[46, 51, 48, 255]);
                }
                let pixels =
                    unsafe { std::slice::from_raw_parts(light.layer.bits, width * height * 4) };
                for (at, source) in pixels.chunks_exact(4).enumerate() {
                    let x = light.layer.rect.left - monitor.left + (at % width) as i32;
                    let y = light.layer.rect.top - monitor.top + (at / width) as i32;
                    let out = (y as usize * 800 + x as usize) * 4;
                    let alpha = u16::from(source[3]) * u16::from(light.opacity) / 255;
                    for c in 0..3 {
                        rgba[out + c] = ((u16::from(source[2 - c]) * u16::from(light.opacity)
                            + u16::from(rgba[out + c]) * (255 - alpha))
                            / 255) as u8;
                    }
                }
                let root = std::path::PathBuf::from(root);
                std::fs::create_dir_all(&root).unwrap();
                crate::png::save_rgba(root.join(format!("spotlight-{index}.png")), 800, 600, &rgba)
                    .unwrap();
            }
        }
        assert!(!light.frame(100.0 + DURATION).unwrap());
        drop(light);
        assert!(!unsafe { IsWindow(hwnd) }.as_bool());
        unsafe {
            DestroyWindow(owner).unwrap();
        }
    }
}
