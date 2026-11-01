//! Everything that touches the real machine goes through [`Platform`].
//!
//! * `Win32Platform` - the real one (Windows only).
//! * `NullPlatform` - finds nothing, sends nothing (non-Windows builds, dry runs).
//! * `MockPlatform` - records every call; used by tests.
//!
//! All methods take `&self`: capture is called from many worker threads at
//! once, while input is serialised by the [`crate::input::Arbiter`], which is
//! the only caller of the input methods.

use crate::capture::Frame;
use crate::geom::{Geometry, Hwnd, Rect};
use crate::keys::Key;
use anyhow::Result;

pub mod file;
pub mod mock;
pub mod null;
#[cfg(windows)]
pub mod win32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WinInfo {
    pub hwnd: Hwnd,
    pub pid: u32,
    pub title: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

pub trait Platform: Send + Sync {
    /// Visible top-level windows of `process_name`, sorted by pid (stable order).
    fn list_windows(&self, process_name: &str) -> Vec<WinInfo>;
    fn geometry(&self, hwnd: Hwnd) -> Option<Geometry>;
    fn foreground(&self) -> Option<Hwnd>;
    /// Restore if minimised, bring to front and take keyboard focus.
    fn focus(&self, hwnd: Hwnd) -> Result<()>;
    fn is_minimized(&self, hwnd: Hwnd) -> bool;
    /// Move/resize the OUTER window rect (screen coords).
    fn move_resize(&self, hwnd: Hwnd, outer: Rect) -> Result<()>;
    /// Primary monitor work area (screen minus taskbar).
    fn work_area(&self) -> Rect;
    /// DPI of the window's monitor (96 = 100 % scaling).
    fn dpi(&self, hwnd: Hwnd) -> u32;
    /// Client-area picture of the window, also when covered by other windows.
    fn capture_client(&self, hwnd: Hwnd) -> Result<Frame>;

    // ---- input (arbiter only) ----
    fn key(&self, key: Key, down: bool) -> Result<()>;
    /// Absolute cursor move to screen (x, y): real move event + exact position.
    fn mouse_move_abs(&self, x: i32, y: i32) -> Result<()>;
    /// Relative motion (first-person camera reads this).
    fn mouse_move_rel(&self, dx: i32, dy: i32) -> Result<()>;
    fn mouse_button(&self, button: MouseButton, down: bool) -> Result<()>;
    /// Wheel notches, positive = up / zoom in.
    fn wheel(&self, notches: i32) -> Result<()>;
}

/// The real platform for this OS, or PNG-backed fake clients when
/// `AA_FAKE_CLIENTS=a.png;b.png` is set (testing without Roblox).
pub fn native() -> Box<dyn Platform> {
    if let Ok(list) = std::env::var("AA_FAKE_CLIENTS") {
        match file::FilePlatform::from_env_list(&list) {
            Ok(p) => return Box::new(p),
            Err(e) => log::error!("AA_FAKE_CLIENTS: {e}"),
        }
    }
    #[cfg(windows)]
    {
        Box::new(win32::Win32Platform::new())
    }
    #[cfg(not(windows))]
    {
        Box::new(null::NullPlatform)
    }
}
