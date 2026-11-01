//! Fake clients backed by PNG files: run the app without Roblox
//! (`AA_FAKE_CLIENTS=a.png;b.png aa-macro`). Input is accepted and ignored.

use super::{MouseButton, Platform, WinInfo};
use crate::capture::Frame;
use crate::geom::{Geometry, Hwnd, Rect};
use crate::keys::Key;
use anyhow::{anyhow, Result};

pub struct FilePlatform {
    frames: Vec<Frame>,
}

impl FilePlatform {
    pub fn from_env_list(list: &str) -> Result<Self> {
        let frames = list
            .split(';')
            .filter(|s| !s.trim().is_empty())
            .map(|p| Frame::load_png(std::path::Path::new(p.trim())))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self { frames })
    }
    fn idx(&self, h: Hwnd) -> Option<usize> {
        let i = (h.0 - 1) as usize;
        (i < self.frames.len()).then_some(i)
    }
}

impl Platform for FilePlatform {
    fn list_windows(&self, _: &str) -> Vec<WinInfo> {
        (0..self.frames.len())
            .map(|i| WinInfo {
                hwnd: Hwnd(i as isize + 1),
                pid: 1 + i as u32,
                title: "Roblox (file)".into(),
            })
            .collect()
    }
    fn geometry(&self, h: Hwnd) -> Option<Geometry> {
        let i = self.idx(h)?;
        let f = &self.frames[i];
        let x = i as i32 * 816;
        Some(Geometry {
            hwnd: h,
            outer: Rect::new(x, 0, f.width as i32 + 16, f.height as i32 + 39),
            client: Rect::new(x + 8, 31, f.width as i32, f.height as i32),
        })
    }
    fn foreground(&self) -> Option<Hwnd> {
        None
    }
    fn focus(&self, _: Hwnd) -> Result<()> {
        Ok(())
    }
    fn is_minimized(&self, _: Hwnd) -> bool {
        false
    }
    fn move_resize(&self, _: Hwnd, _: Rect) -> Result<()> {
        Ok(())
    }
    fn work_area(&self) -> Rect {
        Rect::new(0, 0, 1920, 1040)
    }
    fn dpi(&self, _: Hwnd) -> u32 {
        96
    }
    fn capture_client(&self, h: Hwnd) -> Result<Frame> {
        self.idx(h)
            .map(|i| self.frames[i].clone())
            .ok_or_else(|| anyhow!("no such fake client"))
    }
    fn key(&self, _: Key, _: bool) -> Result<()> {
        Ok(())
    }
    fn mouse_move_abs(&self, _: i32, _: i32) -> Result<()> {
        Ok(())
    }
    fn mouse_move_rel(&self, _: i32, _: i32) -> Result<()> {
        Ok(())
    }
    fn mouse_button(&self, _: MouseButton, _: bool) -> Result<()> {
        Ok(())
    }
    fn wheel(&self, _: i32) -> Result<()> {
        Ok(())
    }
}
