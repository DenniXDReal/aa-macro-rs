//! A captured client-area frame (BGRA, top-down rows).

use anyhow::{Context, Result};
use std::path::Path;

#[derive(Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// BGRA, `width * height * 4` bytes, row 0 = top.
    pub bgra: Vec<u8>,
}

impl std::fmt::Debug for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Frame({}x{})", self.width, self.height)
    }
}

impl Frame {
    pub fn new(width: u32, height: u32, bgra: Vec<u8>) -> Self {
        debug_assert_eq!(bgra.len(), (width * height * 4) as usize);
        Self {
            width,
            height,
            bgra,
        }
    }

    pub fn solid(width: u32, height: u32, b: u8, g: u8, r: u8) -> Self {
        let mut v = Vec::with_capacity((width * height * 4) as usize);
        for _ in 0..width * height {
            v.extend_from_slice(&[b, g, r, 255]);
        }
        Self::new(width, height, v)
    }

    /// (r, g, b) at client pixel, `None` outside.
    pub fn rgb(&self, x: i32, y: i32) -> Option<(u8, u8, u8)> {
        if x < 0 || y < 0 || x as u32 >= self.width || y as u32 >= self.height {
            return None;
        }
        let i = ((y as u32 * self.width + x as u32) * 4) as usize;
        Some((self.bgra[i + 2], self.bgra[i + 1], self.bgra[i]))
    }

    /// True when every pixel is (near) black: PrintWindow failed silently.
    pub fn is_blank(&self) -> bool {
        self.bgra
            .chunks_exact(4)
            .step_by(97)
            .all(|p| p[0] < 8 && p[1] < 8 && p[2] < 8)
    }

    /// Crop a sub-rectangle (clipped to the frame).
    pub fn crop(&self, x: u32, y: u32, w: u32, h: u32) -> Frame {
        let x2 = (x + w).min(self.width);
        let y2 = (y + h).min(self.height);
        let (w, h) = (x2.saturating_sub(x), y2.saturating_sub(y));
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for row in y..y + h {
            let s = ((row * self.width + x) * 4) as usize;
            out.extend_from_slice(&self.bgra[s..s + (w * 4) as usize]);
        }
        Frame::new(w, h, out)
    }

    pub fn from_png_bytes(bytes: &[u8]) -> Result<Frame> {
        let img = image::load_from_memory(bytes)?.to_rgba8();
        let (w, h) = img.dimensions();
        let mut bgra = img.into_raw();
        for p in bgra.chunks_exact_mut(4) {
            p.swap(0, 2);
        }
        Ok(Frame::new(w, h, bgra))
    }

    pub fn load_png(path: &Path) -> Result<Frame> {
        let img = image::open(path)
            .with_context(|| format!("opening {}", path.display()))?
            .to_rgba8();
        let (w, h) = img.dimensions();
        let mut bgra = img.into_raw();
        for p in bgra.chunks_exact_mut(4) {
            p.swap(0, 2);
        }
        Ok(Frame::new(w, h, bgra))
    }

    pub fn save_png(&self, path: &Path) -> Result<()> {
        let mut rgba = self.bgra.clone();
        for p in rgba.chunks_exact_mut(4) {
            p.swap(0, 2);
            p[3] = 255;
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        image::save_buffer(
            path,
            &rgba,
            self.width,
            self.height,
            image::ColorType::Rgba8,
        )
        .with_context(|| format!("saving {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixel_and_crop() {
        let mut f = Frame::solid(4, 3, 0, 0, 0);
        assert!(f.is_blank());
        let i = ((1 * 4 + 2) * 4) as usize; // (2,1)
        f.bgra[i..i + 3].copy_from_slice(&[10, 20, 30]);
        assert_eq!(f.rgb(2, 1), Some((30, 20, 10)));
        assert_eq!(f.rgb(4, 0), None);
        let c = f.crop(2, 1, 5, 5);
        assert_eq!((c.width, c.height), (2, 2));
        assert_eq!(c.rgb(0, 0), Some((30, 20, 10)));
    }
}
