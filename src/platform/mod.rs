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

    /// Mouse cursor, screen coords.
    fn cursor(&self) -> Option<crate::geom::Point> {
        None
    }
    /// This app's own main window (for docking Roblox into it).
    fn own_window(&self) -> Option<Hwnd> {
        None
    }
    /// Put `hwnd` on top of the normal windows without activating it.
    fn raise(&self, _hwnd: Hwnd) -> Result<()> {
        Ok(())
    }
    /// Make `hwnd` owned by `owner` (an owned window always stays above its
    /// owner). `None` removes the owner.
    fn set_owner(&self, _hwnd: Hwnd, _owner: Option<Hwnd>) -> Result<()> {
        Ok(())
    }
    /// Close every process with this exe name (force).
    fn kill_process(&self, _exe: &str) -> Result<u32> {
        Ok(0)
    }
    /// Stop Windows from sleeping / turning the screen off / locking on
    /// idle while the macro runs (input can't reach a locked desktop).
    fn keep_awake(&self, _on: bool) {}
    /// Open a link with whatever handles it (roblox-player: -> Roblox).
    fn open_uri(&self, _uri: &str) -> Result<()> {
        anyhow::bail!("opening links needs Windows")
    }
    /// Read the text in `area` of `frame` (Windows' built-in OCR).
    fn ocr(&self, _frame: &Frame, _area: Rect) -> Option<String> {
        None
    }
    /// OCR with positions: one entry per text line, each word with its box
    /// (in `frame` coordinates). Used to find buttons by their label.
    fn ocr_lines(&self, _frame: &Frame, _area: Rect) -> Option<Vec<Vec<(String, Rect)>>> {
        None
    }
    /// When a process started (to find its Roblox log).
    fn process_started(&self, _pid: u32) -> Option<std::time::SystemTime> {
        None
    }
    /// Roblox runs as administrator and we don't: Windows then blocks moving
    /// it, capturing it and sending it input.
    fn needs_admin(&self, _hwnd: Hwnd) -> bool {
        false
    }
    /// Keep `hwnd` above every normal window (`false` drops that again).
    fn set_topmost(&self, _hwnd: Hwnd, _on: bool) -> Result<()> {
        Ok(())
    }
    /// Put `hwnd` right behind `above` in the z-order.
    fn place_below(&self, _hwnd: Hwnd, _above: Hwnd) -> Result<()> {
        Ok(())
    }
    /// Send `hwnd` behind every other window (stays rendered, still capturable).
    fn lower(&self, _hwnd: Hwnd) -> Result<()> {
        Ok(())
    }

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
