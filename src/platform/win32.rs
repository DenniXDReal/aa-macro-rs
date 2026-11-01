//! Real Windows backend: window discovery, focus, SendInput, PrintWindow capture.

use super::{MouseButton, Platform, WinInfo};
use crate::capture::Frame;
use crate::geom::{Geometry, Hwnd, Point, Rect};
use crate::keys::Key;
use anyhow::{anyhow, bail, Context, Result};
use std::ffi::c_void;
use std::time::{Duration, Instant};

use windows::core::{BOOL, HSTRING, PWSTR};
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, HWND, LPARAM, POINT, RECT,
};
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC,
    GetDIBits, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
};
use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
use windows::Win32::System::Threading::{
    AttachThreadInput, CreateMutexW, GetCurrentThreadId, OpenProcess, QueryFullProcessImageNameW,
    PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::HiDpi::{
    GetDpiForWindow, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, MOUSEEVENTF_ABSOLUTE,
    MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP,
    MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK,
    MOUSEEVENTF_WHEEL, MOUSEINPUT, MOUSE_EVENT_FLAGS, VIRTUAL_KEY, VK_MENU,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumWindows, GetClientRect, GetForegroundWindow, GetSystemMetrics, GetWindow,
    GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
    SetCursorPos, SetForegroundWindow, SetWindowPos, ShowWindow, SystemParametersInfoW, GW_OWNER,
    PW_RENDERFULLCONTENT, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
    SM_YVIRTUALSCREEN, SPI_GETWORKAREA, SWP_NOACTIVATE, SWP_NOZORDER, SW_RESTORE,
    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, WHEEL_DELTA,
};

fn h(w: Hwnd) -> HWND {
    HWND(w.0 as *mut c_void)
}
fn w(h: HWND) -> Hwnd {
    Hwnd(h.0 as isize)
}

/// Make screen / window coordinates physical pixels (call once at startup).
pub fn make_dpi_aware() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

/// Hold named kernel objects so Roblox's single-instance check passes and
/// more clients can open. Keep the returned handles alive.
pub fn hold_singleton(names: &[String]) -> Result<Vec<SingletonHandle>> {
    let mut out = vec![];
    for n in names {
        let handle = unsafe { CreateMutexW(None, true, &HSTRING::from(n.as_str())) }
            .with_context(|| format!("CreateMutex {n}"))?;
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            log::warn!("{n} already exists - a Roblox client is probably already open; close all Roblox first");
        }
        out.push(SingletonHandle(handle));
    }
    Ok(out)
}

pub struct SingletonHandle(HANDLE);
impl Drop for SingletonHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

pub struct Win32Platform;

impl Win32Platform {
    pub fn new() -> Self {
        Win32Platform
    }
}

impl Default for Win32Platform {
    fn default() -> Self {
        Self::new()
    }
}

unsafe extern "system" fn collect(hwnd: HWND, lp: LPARAM) -> BOOL {
    let v = &mut *(lp.0 as *mut Vec<HWND>);
    v.push(hwnd);
    BOOL(1)
}

fn process_exe(pid: u32) -> Option<String> {
    unsafe {
        let ph = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 520];
        let mut len = buf.len() as u32;
        let ok =
            QueryFullProcessImageNameW(ph, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len)
                .is_ok();
        let _ = CloseHandle(ph);
        if !ok {
            return None;
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        path.rsplit(['\\', '/']).next().map(str::to_string)
    }
}

fn title(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetWindowTextW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

fn send(inputs: &[INPUT]) -> Result<()> {
    let sent = unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent as usize != inputs.len() {
        bail!(
            "SendInput sent {sent}/{} (blocked by UIPI? run as the same user as Roblox)",
            inputs.len()
        );
    }
    Ok(())
}

fn mouse(dx: i32, dy: i32, data: u32, flags: MOUSE_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn vk(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: if up {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

impl Platform for Win32Platform {
    fn list_windows(&self, process_name: &str) -> Vec<WinInfo> {
        let mut all: Vec<HWND> = vec![];
        unsafe {
            let _ = EnumWindows(Some(collect), LPARAM(&mut all as *mut Vec<HWND> as isize));
        }
        let mut out = vec![];
        for hwnd in all {
            unsafe {
                if !IsWindowVisible(hwnd).as_bool() {
                    continue;
                }
                // Skip owned popups / tool windows.
                if GetWindow(hwnd, GW_OWNER)
                    .map(|o| !o.0.is_null())
                    .unwrap_or(false)
                {
                    continue;
                }
                let mut pid = 0u32;
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
                let Some(exe) = process_exe(pid) else {
                    continue;
                };
                if !exe.eq_ignore_ascii_case(process_name) {
                    continue;
                }
                let mut r = RECT::default();
                if GetWindowRect(hwnd, &mut r).is_err() || r.right - r.left < 100 {
                    continue;
                }
                out.push(WinInfo {
                    hwnd: w(hwnd),
                    pid,
                    title: title(hwnd),
                });
            }
        }
        out.sort_by_key(|i| (i.pid, i.hwnd));
        out
    }

    fn geometry(&self, hwnd: Hwnd) -> Option<Geometry> {
        unsafe {
            let hw = h(hwnd);
            let mut outer = RECT::default();
            GetWindowRect(hw, &mut outer).ok()?;
            let mut cr = RECT::default();
            GetClientRect(hw, &mut cr).ok()?;
            let mut origin = POINT { x: 0, y: 0 };
            if !ClientToScreen(hw, &mut origin).as_bool() {
                return None;
            }
            Some(Geometry {
                hwnd,
                outer: Rect::new(
                    outer.left,
                    outer.top,
                    outer.right - outer.left,
                    outer.bottom - outer.top,
                ),
                client: Rect::new(origin.x, origin.y, cr.right - cr.left, cr.bottom - cr.top),
            })
        }
    }

    fn foreground(&self) -> Option<Hwnd> {
        let f = unsafe { GetForegroundWindow() };
        (!f.0.is_null()).then(|| w(f))
    }

    fn focus(&self, hwnd: Hwnd) -> Result<()> {
        let hw = h(hwnd);
        unsafe {
            if IsIconic(hw).as_bool() {
                let _ = ShowWindow(hw, SW_RESTORE);
                std::thread::sleep(Duration::from_millis(150));
            }
            if GetForegroundWindow() == hw {
                return Ok(());
            }
            // Windows only lets a process steal focus in some cases. Attach to the
            // current foreground thread's input queue, then fall back to an Alt tap.
            let fg = GetForegroundWindow();
            let fg_thread = GetWindowThreadProcessId(fg, None);
            let me = GetCurrentThreadId();
            let attached = fg_thread != 0
                && fg_thread != me
                && AttachThreadInput(me, fg_thread, true).as_bool();
            let _ = BringWindowToTop(hw);
            let mut ok = SetForegroundWindow(hw).as_bool();
            if attached {
                let _ = AttachThreadInput(me, fg_thread, false);
            }
            if !ok {
                send(&[vk(VK_MENU, false), vk(VK_MENU, true)])?;
                ok = SetForegroundWindow(hw).as_bool();
            }
            let t = Instant::now();
            while t.elapsed() < Duration::from_millis(600) {
                if GetForegroundWindow() == hw {
                    // Roblox needs a moment before it accepts input after activation.
                    std::thread::sleep(Duration::from_millis(40));
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(anyhow!(
                "window did not come to the front (SetForegroundWindow={ok})"
            ))
        }
    }

    fn is_minimized(&self, hwnd: Hwnd) -> bool {
        unsafe { IsIconic(h(hwnd)).as_bool() }
    }

    fn move_resize(&self, hwnd: Hwnd, r: Rect) -> Result<()> {
        unsafe {
            let hw = h(hwnd);
            if IsIconic(hw).as_bool() {
                let _ = ShowWindow(hw, SW_RESTORE);
            }
            SetWindowPos(hw, None, r.x, r.y, r.w, r.h, SWP_NOZORDER | SWP_NOACTIVATE)
                .context("SetWindowPos")
        }
    }

    fn work_area(&self) -> Rect {
        let mut r = RECT::default();
        unsafe {
            if SystemParametersInfoW(
                SPI_GETWORKAREA,
                0,
                Some(&mut r as *mut RECT as *mut c_void),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
            .is_ok()
            {
                return Rect::new(r.left, r.top, r.right - r.left, r.bottom - r.top);
            }
        }
        Rect::new(0, 0, 1920, 1040)
    }

    fn dpi(&self, hwnd: Hwnd) -> u32 {
        match unsafe { GetDpiForWindow(h(hwnd)) } {
            0 => 96,
            d => d,
        }
    }

    fn capture_client(&self, hwnd: Hwnd) -> Result<Frame> {
        let g = self.geometry(hwnd).ok_or_else(|| anyhow!("window gone"))?;
        let (ow, oh) = (g.outer.w, g.outer.h);
        if ow <= 0 || oh <= 0 || g.client.w <= 0 || g.client.h <= 0 {
            bail!("window has no size (minimised?)");
        }
        let hw = h(hwnd);
        unsafe {
            let screen = GetDC(None);
            let mem = CreateCompatibleDC(Some(screen));
            let bmp = CreateCompatibleBitmap(screen, ow, oh);
            let old = SelectObject(mem, bmp.into());
            // PW_RENDERFULLCONTENT: works for DirectX windows and when covered.
            let ok = PrintWindow(hw, mem, PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT)).as_bool();
            let mut info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: ow,
                    biHeight: -oh, // top-down rows
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut buf = vec![0u8; (ow * oh * 4) as usize];
            let lines = GetDIBits(
                mem,
                bmp,
                0,
                oh as u32,
                Some(buf.as_mut_ptr() as *mut c_void),
                &mut info,
                DIB_RGB_COLORS,
            );
            SelectObject(mem, old);
            let _ = DeleteObject(bmp.into());
            let _ = DeleteDC(mem);
            ReleaseDC(None, screen);
            if !ok {
                bail!("PrintWindow failed");
            }
            if lines != oh {
                bail!("GetDIBits copied {lines}/{oh} lines");
            }
            let full = Frame::new(ow as u32, oh as u32, buf);
            let (bx, by) = g.border();
            Ok(full.crop(
                bx.max(0) as u32,
                by.max(0) as u32,
                g.client.w as u32,
                g.client.h as u32,
            ))
        }
    }

    fn key(&self, key: Key, down: bool) -> Result<()> {
        let mut flags = KEYEVENTF_SCANCODE;
        if key.extended {
            flags |= KEYEVENTF_EXTENDEDKEY;
        }
        if !down {
            flags |= KEYEVENTF_KEYUP;
        }
        send(&[INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0),
                    wScan: key.scan,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }])
    }

    fn mouse_move_abs(&self, x: i32, y: i32) -> Result<()> {
        // A real move event (so Roblox updates hover), then an exact SetCursorPos
        // because the 0..65535 absolute scale can land one pixel off.
        let (vx, vy, vw, vh) = unsafe {
            (
                GetSystemMetrics(SM_XVIRTUALSCREEN),
                GetSystemMetrics(SM_YVIRTUALSCREEN),
                GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1),
                GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1),
            )
        };
        let nx = (((x - vx) as i64 * 65535) / (vw - 1).max(1) as i64) as i32;
        let ny = (((y - vy) as i64 * 65535) / (vh - 1).max(1) as i64) as i32;
        send(&[mouse(
            nx,
            ny,
            0,
            MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
        )])?;
        unsafe { SetCursorPos(x, y) }.context("SetCursorPos")?;
        Ok(())
    }

    fn mouse_move_rel(&self, dx: i32, dy: i32) -> Result<()> {
        send(&[mouse(dx, dy, 0, MOUSEEVENTF_MOVE)])
    }

    fn mouse_button(&self, b: MouseButton, down: bool) -> Result<()> {
        let f = match (b, down) {
            (MouseButton::Left, true) => MOUSEEVENTF_LEFTDOWN,
            (MouseButton::Left, false) => MOUSEEVENTF_LEFTUP,
            (MouseButton::Right, true) => MOUSEEVENTF_RIGHTDOWN,
            (MouseButton::Right, false) => MOUSEEVENTF_RIGHTUP,
            (MouseButton::Middle, true) => MOUSEEVENTF_MIDDLEDOWN,
            (MouseButton::Middle, false) => MOUSEEVENTF_MIDDLEUP,
        };
        send(&[mouse(0, 0, 0, f)])
    }

    fn wheel(&self, notches: i32) -> Result<()> {
        let data = (notches * WHEEL_DELTA as i32) as u32;
        send(&[mouse(0, 0, data, MOUSEEVENTF_WHEEL)])
    }
}

/// Current cursor position (screen), for diagnostics.
pub fn cursor() -> Option<Point> {
    let mut p = POINT::default();
    unsafe { windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut p) }.ok()?;
    Some(Point::new(p.x, p.y))
}
