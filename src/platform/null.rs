use super::{MouseButton, Platform, WinInfo};
use crate::capture::Frame;
use crate::geom::{Geometry, Hwnd, Rect};
use crate::keys::Key;
use anyhow::{bail, Result};

/// Finds nothing, sends nothing. Lets the CLI build and run anywhere.
pub struct NullPlatform;

impl Platform for NullPlatform {
    fn list_windows(&self, _: &str) -> Vec<WinInfo> {
        vec![]
    }
    fn geometry(&self, _: Hwnd) -> Option<Geometry> {
        None
    }
    fn foreground(&self) -> Option<Hwnd> {
        None
    }
    fn focus(&self, _: Hwnd) -> Result<()> {
        bail!("no windows on this platform")
    }
    fn is_minimized(&self, _: Hwnd) -> bool {
        false
    }
    fn move_resize(&self, _: Hwnd, _: Rect) -> Result<()> {
        bail!("no windows on this platform")
    }
    fn work_area(&self) -> Rect {
        Rect::new(0, 0, 1920, 1040)
    }
    fn dpi(&self, _: Hwnd) -> u32 {
        96
    }
    fn capture_client(&self, _: Hwnd) -> Result<Frame> {
        bail!("capture needs Windows")
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
