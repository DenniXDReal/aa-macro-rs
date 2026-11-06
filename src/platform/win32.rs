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

/// Windows' own OCR (the same engine the AHK macro's OCR.ahk uses): the crop
/// is greyed and scaled 4x first, which small game text needs.
fn ocr_run(
    frame: &Frame,
    area: Rect,
    max_k: i32,
) -> Result<(windows::Media::Ocr::OcrResult, Rect, i32)> {
    use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
    use windows::Media::Ocr::OcrEngine;
    use windows::Security::Cryptography::CryptographicBuffer;
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let a = area
        .intersect(&Rect::new(0, 0, frame.width as i32, frame.height as i32))
        .ok_or_else(|| anyhow!("OCR area outside the picture"))?;
    // small text wants 4x; big areas less (Windows OCR caps the picture size)
    let k = (2400 / a.w.max(a.h).max(1)).clamp(1, max_k);
    let (w, h) = (a.w * k, a.h * k);
    let mut buf = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let (r, g, b) = frame.rgb(a.x + x / k, a.y + y / k).unwrap_or((0, 0, 0));
            let v = ((r as u32 * 38 + g as u32 * 75 + b as u32 * 15) >> 7) as u8;
            let i = ((y * w + x) * 4) as usize;
            buf[i..i + 4].copy_from_slice(&[v, v, v, 255]);
        }
    }
    let ib = CryptographicBuffer::CreateFromByteArray(&buf)?;
    let bmp = SoftwareBitmap::CreateCopyFromBuffer(&ib, BitmapPixelFormat::Bgra8, w, h)?;
    // one OCR at a time: several accounts' threads reading at once made the
    // engine fail now and then; and one retry when it does
    static ONE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    // placement checks of different accounts may read at the same time
    // (each its own engine); background reads still go one at a time
    let urgent = super::OCR_IS_URGENT.with(|c| c.get());
    let _g = (!urgent).then(|| ONE.lock().unwrap_or_else(|e| e.into_inner()));
    let run = || -> windows::core::Result<windows::Media::Ocr::OcrResult> {
        // English on purpose (the game is in English): the user-profile
        // language may be another one, or several
        let engine = windows::Globalization::Language::CreateLanguage(
            &windows::core::HSTRING::from("en-US"),
        )
        .and_then(|l| OcrEngine::TryCreateFromLanguage(&l))
        .or_else(|_| OcrEngine::TryCreateFromUserProfileLanguages())?;
        engine.RecognizeAsync(&bmp)?.get()
    };
    let res = match run() {
        Ok(r) => r,
        Err(_) => {
            std::thread::sleep(std::time::Duration::from_millis(60));
            run()?
        }
    };
    Ok((res, a, k))
}

pub fn ocr_text(frame: &Frame, area: Rect) -> Result<String> {
    let (res, _, _) = ocr_run(frame, area, 4)?;
    Ok(res.Text()?.to_string())
}

/// OCR with word boxes, mapped back to `frame` coordinates.
pub fn ocr_lines(frame: &Frame, area: Rect) -> Result<Vec<Vec<(String, Rect)>>> {
    // whole screen (button labels are big): 2x is plenty and much quicker;
    // a small area (e.g. around the unit panel's UPG text): up to 4x
    let big = area.w as i64 * area.h as i64 > 200_000;
    let (res, a, k) = ocr_run(frame, area, if big { 2 } else { 4 })?;
    let mut out = vec![];
    for line in res.Lines()? {
        let mut words = vec![];
        for w in line.Words()? {
            let b = w.BoundingRect()?;
            let r = Rect::new(
                a.x + (b.X / k as f32) as i32,
                a.y + (b.Y / k as f32) as i32,
                ((b.Width / k as f32) as i32).max(1),
                ((b.Height / k as f32) as i32).max(1),
            );
            words.push((w.Text()?.to_string(), r));
        }
        if !words.is_empty() {
            out.push(words);
        }
    }
    Ok(out)
}

/// Is this process running as administrator? `Some(true)` also when we may
/// not even read its token (it runs above us).
pub fn process_elevated(pid: u32) -> Option<bool> {
    use windows::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows::Win32::System::Threading::OpenProcessToken;
    unsafe {
        let hp = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut tok = HANDLE::default();
        let r = OpenProcessToken(hp, TOKEN_QUERY, &mut tok);
        let _ = CloseHandle(hp);
        if r.is_err() {
            return Some(true);
        }
        let mut e = TOKEN_ELEVATION::default();
        let mut len = 0u32;
        let ok = GetTokenInformation(
            tok,
            TokenElevation,
            Some(&mut e as *mut TOKEN_ELEVATION as *mut c_void),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        )
        .is_ok();
        let _ = CloseHandle(tok);
        ok.then_some(e.TokenIsElevated != 0)
    }
}

pub fn self_elevated() -> bool {
    process_elevated(std::process::id()).unwrap_or(false)
}

/// Start this exe again as administrator (UAC prompt). True = it started.
pub fn relaunch_as_admin() -> bool {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let r = unsafe {
        ShellExecuteW(
            None,
            &HSTRING::from("runas"),
            &HSTRING::from(exe.as_os_str()),
            windows::core::PCWSTR::null(),
            windows::core::PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    r.0 as isize > 32
}

/// Make screen / window coordinates physical pixels (call once at startup).
pub fn make_dpi_aware() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

/// Hold named kernel objects so Roblox's single-instance check passes and
/// more clients can open. Keep the returned handles alive. The flag is true
/// when the object already existed (an open Roblox, or RAM's own lock).
pub fn hold_singleton(names: &[String]) -> Result<(Vec<SingletonHandle>, bool)> {
    let mut out = vec![];
    let mut existed = false;
    for n in names {
        let handle = unsafe { CreateMutexW(None, true, &HSTRING::from(n.as_str())) }
            .with_context(|| format!("CreateMutex {n}"))?;
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            existed = true;
            // take ownership if nobody holds it (left over / abandoned) - RAM does the same
            unsafe {
                let _ = windows::Win32::System::Threading::WaitForSingleObject(handle, 0);
            }
        }
        out.push(SingletonHandle(handle));
    }
    Ok((out, existed))
}

pub struct SingletonHandle(HANDLE);
// A kernel handle is just a number; closing it from another thread is fine.
unsafe impl Send for SingletonHandle {}
impl Drop for SingletonHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

pub struct Win32Platform;

impl Win32Platform {
    /// The docked app keeps one Roblox client topmost. When input goes to a
    /// different client of the same game, hand "topmost" over to it so it
    /// really is in front for the clicks.
    fn take_topmost(&self, hwnd: Hwnd) {
        use windows::Win32::UI::WindowsAndMessaging::{
            GetWindowLongPtrW, GWL_EXSTYLE, WS_EX_TOPMOST,
        };
        let topmost = |x: Hwnd| unsafe {
            (GetWindowLongPtrW(h(x), GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0) != 0
        };
        let mut pid = 0u32;
        unsafe {
            GetWindowThreadProcessId(h(hwnd), Some(&mut pid));
        }
        let Some(exe) = process_exe(pid) else { return };
        let others: Vec<Hwnd> = self
            .list_windows(&exe)
            .into_iter()
            .map(|i| i.hwnd)
            .filter(|x| *x != hwnd && topmost(*x))
            .collect();
        if others.is_empty() {
            return;
        }
        for o in others {
            let _ = self.set_topmost(o, false);
        }
        let _ = self.set_topmost(hwnd, true);
    }

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

/// Running inside a Remote Desktop session?
pub fn remote_session() -> bool {
    unsafe { GetSystemMetrics(windows::Win32::UI::WindowsAndMessaging::SM_REMOTESESSION) != 0 }
}

fn send(inputs: &[INPUT]) -> Result<()> {
    let mut sent = unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
    // Windows refuses input for a moment now and then (focus change, a
    // notification popping up): try again before calling it blocked
    let mut tries = 0;
    while sent == 0 && tries < 5 {
        std::thread::sleep(std::time::Duration::from_millis(40 * (tries + 1)));
        sent = unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
        tries += 1;
    }
    if sent as usize != inputs.len() {
        let err = unsafe { GetLastError() }.0;
        // SM_REMOTESESSION (0x1000): running inside Remote Desktop
        let rdp =
            unsafe { GetSystemMetrics(windows::Win32::UI::WindowsAndMessaging::SM_REMOTESESSION) }
                != 0;
        if rdp {
            bail!(
                "SendInput sent {sent}/{} (error {err}) - input blocked: this PC is used over Remote Desktop and the RDP window is minimized or disconnected (Windows stops all input then). Keep the RDP window open, or see Settings > Remote Desktop",
                inputs.len()
            );
        }
        bail!(
            "SendInput sent {sent}/{} (error {err}) - input blocked (PC locked / screen off / UAC prompt, or Roblox runs as admin)",
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
        self.take_topmost(hwnd);
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

    fn cursor(&self) -> Option<Point> {
        cursor()
    }

    fn own_window(&self) -> Option<Hwnd> {
        use windows::Win32::System::Threading::GetCurrentProcessId;
        let me = unsafe { GetCurrentProcessId() };
        let mut all: Vec<HWND> = vec![];
        unsafe {
            let _ = EnumWindows(Some(collect), LPARAM(&mut all as *mut Vec<HWND> as isize));
        }
        all.into_iter()
            .filter(|h| unsafe {
                let mut pid = 0u32;
                GetWindowThreadProcessId(*h, Some(&mut pid));
                pid == me
                    && IsWindowVisible(*h).as_bool()
                    && !GetWindow(*h, GW_OWNER)
                        .map(|o| !o.0.is_null())
                        .unwrap_or(false)
            })
            .max_by_key(|h| {
                let mut r = RECT::default();
                unsafe {
                    let _ = GetWindowRect(*h, &mut r);
                }
                (r.right - r.left) * (r.bottom - r.top)
            })
            .map(w)
    }

    fn set_owner(&self, hwnd: Hwnd, owner: Option<Hwnd>) -> Result<()> {
        use windows::Win32::UI::WindowsAndMessaging::{SetWindowLongPtrW, GWLP_HWNDPARENT};
        unsafe {
            SetWindowLongPtrW(h(hwnd), GWLP_HWNDPARENT, owner.map(|o| o.0).unwrap_or(0));
        }
        Ok(())
    }

    fn kill_process(&self, exe: &str) -> Result<u32> {
        if exe.contains(['\\', '/', '"', '&', '|', '<', '>']) {
            bail!("bad process name");
        }
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let out = std::process::Command::new("taskkill")
            .args(["/F", "/IM", exe])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .context("taskkill")?;
        let text = String::from_utf8_lossy(&out.stdout);
        Ok(text.matches("SUCCESS").count() as u32)
    }

    fn kill_pid(&self, pid: u32) -> Result<()> {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::process::Command::new("taskkill")
            .args(["/F", "/PID", &pid.to_string()])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .context("taskkill")?;
        Ok(())
    }

    fn raise(&self, hwnd: Hwnd) -> Result<()> {
        use windows::Win32::UI::WindowsAndMessaging::{HWND_TOP, SWP_NOMOVE, SWP_NOSIZE};
        unsafe {
            SetWindowPos(
                h(hwnd),
                Some(HWND_TOP),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
        }
        .context("raise")
    }

    fn keep_awake(&self, on: bool) {
        use windows::Win32::System::Power::{
            SetThreadExecutionState, ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED,
        };
        unsafe {
            let _ = SetThreadExecutionState(if on {
                ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED
            } else {
                ES_CONTINUOUS
            });
        }
    }

    fn open_uri(&self, uri: &str) -> Result<()> {
        use std::os::windows::process::CommandExt;
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        if !uri.starts_with("roblox-player:") {
            bail!("only roblox-player: links are opened");
        }
        // ShellExecute hands links to their app through COM: init it on this thread
        unsafe {
            use windows::Win32::System::Com::{
                CoInitializeEx, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
            };
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        }
        let r = unsafe {
            ShellExecuteW(
                None,
                &HSTRING::from("open"),
                &HSTRING::from(uri),
                windows::core::PCWSTR::null(),
                windows::core::PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        if (r.0 as isize) > 32 {
            return Ok(());
        }
        // fallback: run the registered Roblox launcher with the link
        let out = std::process::Command::new("reg")
            .args(["query", r"HKCR\roblox-player\shell\open\command", "/ve"])
            .creation_flags(0x0800_0000)
            .output()
            .context("reading the roblox-player handler")?;
        let text = String::from_utf8_lossy(&out.stdout);
        let exe = text
            .split('"')
            .find(|p| p.to_lowercase().ends_with(".exe"))
            .map(|s| s.to_string())
            .ok_or_else(|| {
                anyhow!(
                    "Roblox isn't installed (no roblox-player handler) - ShellExecute said {}",
                    r.0 as isize
                )
            })?;
        std::process::Command::new(&exe)
            .arg(uri)
            .spawn()
            .with_context(|| format!("starting {exe}"))?;
        Ok(())
    }

    fn has_ocr(&self) -> bool {
        true
    }

    fn is_remote_session(&self) -> bool {
        remote_session()
    }

    fn ocr(&self, frame: &Frame, area: Rect) -> Option<String> {
        ocr_text(frame, area).ok()
    }

    fn ocr_lines(&self, frame: &Frame, area: Rect) -> Option<Vec<Vec<(String, Rect)>>> {
        ocr_lines(frame, area).ok()
    }

    fn process_started(&self, pid: u32) -> Option<std::time::SystemTime> {
        use windows::Win32::Foundation::FILETIME;
        use windows::Win32::System::Threading::GetProcessTimes;
        unsafe {
            let hp = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
            let (mut c, mut e, mut k, mut u) = (
                FILETIME::default(),
                FILETIME::default(),
                FILETIME::default(),
                FILETIME::default(),
            );
            let ok = GetProcessTimes(hp, &mut c, &mut e, &mut k, &mut u).is_ok();
            let _ = CloseHandle(hp);
            if !ok {
                return None;
            }
            let t = ((c.dwHighDateTime as u64) << 32) | c.dwLowDateTime as u64;
            // 100 ns ticks since 1601 -> unix
            let unix_100ns = t.checked_sub(116_444_736_000_000_000)?;
            Some(std::time::UNIX_EPOCH + std::time::Duration::from_nanos(unix_100ns * 100))
        }
    }

    fn needs_admin(&self, hwnd: Hwnd) -> bool {
        let mut pid = 0u32;
        unsafe {
            GetWindowThreadProcessId(h(hwnd), Some(&mut pid));
        }
        process_elevated(pid) == Some(true) && !self_elevated()
    }

    fn set_topmost(&self, hwnd: Hwnd, on: bool) -> Result<()> {
        use windows::Win32::UI::WindowsAndMessaging::{
            HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOMOVE, SWP_NOSIZE,
        };
        unsafe {
            SetWindowPos(
                h(hwnd),
                Some(if on { HWND_TOPMOST } else { HWND_NOTOPMOST }),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
        }
        .context("topmost")
    }

    fn place_below(&self, hwnd: Hwnd, above: Hwnd) -> Result<()> {
        use windows::Win32::UI::WindowsAndMessaging::{SWP_NOMOVE, SWP_NOSIZE};
        unsafe {
            SetWindowPos(
                h(hwnd),
                Some(h(above)),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
        }
        .context("place below")
    }

    fn lower(&self, hwnd: Hwnd) -> Result<()> {
        use windows::Win32::UI::WindowsAndMessaging::{HWND_BOTTOM, SWP_NOMOVE, SWP_NOSIZE};
        unsafe {
            SetWindowPos(
                h(hwnd),
                Some(HWND_BOTTOM),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
        }
        .context("lower")
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
                .map_err(|e| anyhow!("SetWindowPos: {}", e.message()))
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
                bail!(
                    "PrintWindow failed: {}",
                    windows::core::Error::from_win32().message()
                );
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

/// F1..F4 and F6 as global hotkeys (work while Roblox has focus). Each press sends
/// its number (1-4) down `tx`. The thread lives for the whole app.
pub fn spawn_hotkeys(tx: std::sync::mpsc::Sender<u8>, overlay_fkey: u32) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, MOD_NOREPEAT, VK_F1};
    use windows::Win32::UI::WindowsAndMessaging::{GetMessageW, MSG, WM_HOTKEY};
    std::thread::Builder::new()
        .name("hotkeys".into())
        .spawn(move || unsafe {
            // F1-F4 run controls, F6 hover capture
            // + the debug overlay key (F9 by default; sends its F number)
            for i in [0u32, 1, 2, 3, 5, overlay_fkey - 1] {
                if RegisterHotKey(None, (i + 1) as i32, MOD_NOREPEAT, VK_F1.0 as u32 + i).is_err() {
                    log::warn!("F{} is used by another app - hotkey not available", i + 1);
                }
            }
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                if msg.message == WM_HOTKEY && tx.send(msg.wParam.0 as u8).is_err() {
                    break;
                }
            }
        })
        .ok();
}
