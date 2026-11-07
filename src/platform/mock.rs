//! Test double: fake windows + a log of every call.

use super::{MouseButton, Platform, WinInfo};
use crate::capture::Frame;
use crate::geom::{Geometry, Hwnd, Rect};
use crate::keys::Key;
use anyhow::{bail, Result};
use std::collections::HashSet;
use std::sync::Mutex;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Call {
    Focus(Hwnd),
    Key(Key, bool),
    MoveAbs(i32, i32),
    MoveRel(i32, i32),
    Button(MouseButton, bool),
    Wheel(i32),
    MoveResize(Hwnd, Rect),
    Char(char, bool),
}

#[derive(Default)]
pub struct MockPlatform {
    pub windows: Mutex<Vec<(WinInfo, Geometry)>>,
    pub calls: Mutex<Vec<Call>>,
    pub fg: Mutex<Option<Hwnd>>,
    /// Windows that refuse focus.
    pub refuse_focus: Mutex<HashSet<Hwnd>>,
}

impl MockPlatform {
    /// `n` fake 800x599 clients laid out side by side.
    pub fn with_windows(n: usize) -> Self {
        let m = MockPlatform::default();
        {
            let mut w = m.windows.lock().unwrap();
            for i in 0..n {
                let hwnd = Hwnd(100 + i as isize);
                let x = i as i32 * 816;
                w.push((
                    WinInfo {
                        hwnd,
                        pid: 1000 + i as u32,
                        title: "Roblox".into(),
                    },
                    Geometry {
                        hwnd,
                        outer: Rect::new(x, 0, 816, 638),
                        client: Rect::new(x + 8, 31, 800, 599),
                    },
                ));
            }
        }
        m
    }
    pub fn take_calls(&self) -> Vec<Call> {
        std::mem::take(&mut *self.calls.lock().unwrap())
    }
    fn log(&self, c: Call) {
        self.calls.lock().unwrap().push(c);
    }
}

impl Platform for MockPlatform {
    fn list_windows(&self, _: &str) -> Vec<WinInfo> {
        self.windows
            .lock()
            .unwrap()
            .iter()
            .map(|(i, _)| i.clone())
            .collect()
    }
    fn geometry(&self, hwnd: Hwnd) -> Option<Geometry> {
        self.windows
            .lock()
            .unwrap()
            .iter()
            .find(|(i, _)| i.hwnd == hwnd)
            .map(|(_, g)| *g)
    }
    fn foreground(&self) -> Option<Hwnd> {
        *self.fg.lock().unwrap()
    }
    fn focus(&self, hwnd: Hwnd) -> Result<()> {
        if self.refuse_focus.lock().unwrap().contains(&hwnd) {
            bail!("focus refused");
        }
        self.log(Call::Focus(hwnd));
        *self.fg.lock().unwrap() = Some(hwnd);
        Ok(())
    }
    fn is_minimized(&self, _: Hwnd) -> bool {
        false
    }
    fn move_resize(&self, hwnd: Hwnd, outer: Rect) -> Result<()> {
        self.log(Call::MoveResize(hwnd, outer));
        Ok(())
    }
    fn work_area(&self) -> Rect {
        Rect::new(0, 0, 1920, 1040)
    }
    fn dpi(&self, _: Hwnd) -> u32 {
        96
    }
    fn capture_client(&self, _: Hwnd) -> Result<Frame> {
        Ok(Frame::solid(800, 599, 40, 40, 40))
    }
    fn key(&self, key: Key, down: bool) -> Result<()> {
        self.log(Call::Key(key, down));
        Ok(())
    }
    fn type_char(&self, c: char, down: bool) -> Result<()> {
        self.log(Call::Char(c, down));
        Ok(())
    }
    fn mouse_move_abs(&self, x: i32, y: i32) -> Result<()> {
        self.log(Call::MoveAbs(x, y));
        Ok(())
    }
    fn mouse_move_rel(&self, dx: i32, dy: i32) -> Result<()> {
        self.log(Call::MoveRel(dx, dy));
        Ok(())
    }
    fn mouse_button(&self, b: MouseButton, down: bool) -> Result<()> {
        self.log(Call::Button(b, down));
        Ok(())
    }
    fn wheel(&self, n: i32) -> Result<()> {
        self.log(Call::Wheel(n));
        Ok(())
    }
}
