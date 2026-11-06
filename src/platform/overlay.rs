//! Debug overlay (Windows): a click-through, always-on-top, see-through
//! window over the whole desktop that draws what the macro sees - pattern
//! hits (green) / misses (red), OCR boxes + text (cyan), click targets
//! (crosshair), the current step and each client's input generation and
//! picture age. Toggle with the overlay hotkey (F9 by default).
//!
//! It only draws: everything comes from `debugbus::latest()`, nothing is
//! captured or analysed here. Magenta is the see-through colour key.

use crate::debugbus::{self, MarkKind};
use std::ffi::c_void;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BitBlt, ClientToScreen, CreateCompatibleBitmap, CreateCompatibleDC, CreatePen,
    CreateSolidBrush, DeleteDC, DeleteObject, FillRect, GetDC, GetStockObject, LineTo, MoveToEx,
    Rectangle, ReleaseDC, SelectObject, SetBkMode, SetTextColor, TextOutW, HDC, NULL_BRUSH,
    PS_SOLID, SRCCOPY, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetSystemMetrics, PeekMessageW,
    RegisterClassW, SetLayeredWindowAttributes, SetWindowPos, ShowWindow, TranslateMessage,
    HWND_TOPMOST, LWA_COLORKEY, MSG, PM_REMOVE, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
    SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SWP_NOACTIVATE, SW_HIDE, SW_SHOWNOACTIVATE,
    WM_ERASEBKGND, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
};

const KEY: COLORREF = COLORREF(0x00FF_00FF); // magenta = see-through

fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16)
}

unsafe extern "system" fn wndproc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_ERASEBKGND {
        return LRESULT(1);
    }
    unsafe { DefWindowProcW(h, msg, wp, lp) }
}

/// Start the overlay thread (once). It shows / hides itself following
/// `debugbus::enabled()`.
pub fn spawn() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        let _ = std::thread::Builder::new()
            .name("debug-overlay".into())
            .spawn(|| unsafe { run() });
    });
}

unsafe fn run() {
    unsafe {
        let inst = GetModuleHandleW(PCWSTR::null()).unwrap_or_default();
        let class = w!("AaMacroDebugOverlay");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: inst.into(),
            lpszClassName: class,
            ..Default::default()
        };
        RegisterClassW(&wc);
        let (vx, vy, vw, vh) = (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        );
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            class,
            w!("AA macro debug overlay"),
            WS_POPUP,
            vx,
            vy,
            vw,
            vh,
            None,
            None,
            Some(inst.into()),
            None,
        ) else {
            log::warn!("debug overlay: window could not be created");
            return;
        };
        let _ = SetLayeredWindowAttributes(hwnd, KEY, 255, LWA_COLORKEY);
        let mut shown = false;
        loop {
            // keep the window responsive
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let on = debugbus::enabled();
            if on != shown {
                let _ = ShowWindow(hwnd, if on { SW_SHOWNOACTIVATE } else { SW_HIDE });
                shown = on;
            }
            if on {
                let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), vx, vy, vw, vh, SWP_NOACTIVATE);
                draw(hwnd, vx, vy, vw, vh);
            }
            std::thread::sleep(std::time::Duration::from_millis(if on { 33 } else { 150 }));
        }
    }
}

unsafe fn text(dc: HDC, x: i32, y: i32, s: &str, c: COLORREF) {
    unsafe {
        let w: Vec<u16> = s.encode_utf16().collect();
        // dark outline so it reads on any background
        SetTextColor(dc, rgb(10, 10, 10));
        for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            let _ = TextOutW(dc, x + dx, y + dy, &w);
        }
        SetTextColor(dc, c);
        let _ = TextOutW(dc, x, y, &w);
    }
}

unsafe fn draw(hwnd: HWND, vx: i32, vy: i32, vw: i32, vh: i32) {
    unsafe {
        let snap = debugbus::latest();
        let screen = GetDC(Some(hwnd));
        let mem = CreateCompatibleDC(Some(screen));
        let bmp = CreateCompatibleBitmap(screen, vw, vh);
        let old = SelectObject(mem, bmp.into());
        let bg = CreateSolidBrush(KEY);
        FillRect(
            mem,
            &RECT {
                left: 0,
                top: 0,
                right: vw,
                bottom: vh,
            },
            bg,
        );
        let _ = DeleteObject(bg.into());
        SetBkMode(mem, TRANSPARENT);
        let null_brush = GetStockObject(NULL_BRUSH);
        let old_brush = SelectObject(mem, null_brush);
        // client px -> overlay px
        let origin = |win: crate::geom::Hwnd| -> Option<(i32, i32)> {
            let mut p = POINT { x: 0, y: 0 };
            ClientToScreen(HWND(win.0 as *mut c_void), &mut p)
                .as_bool()
                .then_some((p.x - vx, p.y - vy))
        };
        for c in &snap.clients {
            if let Some((ox, oy)) = origin(c.window) {
                let age = c
                    .capture_age_ms
                    .map(|a| format!("{a} ms"))
                    .unwrap_or_else(|| "-".into());
                text(
                    mem,
                    ox + 4,
                    oy + 4,
                    &format!(
                        "C{}  step {} ({:.1}s)  gen {}  pic {age}",
                        c.n,
                        snap.step,
                        snap.step_ms as f32 / 1000.0,
                        c.generation
                    ),
                    rgb(255, 230, 90),
                );
            }
        }
        for m in &snap.marks {
            let Some((ox, oy)) = origin(m.window) else {
                continue;
            };
            let (x, y) = (ox + m.rect.x, oy + m.rect.y);
            let color = match m.kind {
                MarkKind::Hit => rgb(60, 230, 90),
                MarkKind::Miss => rgb(240, 60, 60),
                MarkKind::Ocr => rgb(80, 210, 255),
                MarkKind::Click => rgb(255, 255, 255),
            };
            let pen = CreatePen(PS_SOLID, 2, color);
            let old_pen = SelectObject(mem, pen.into());
            match m.kind {
                MarkKind::Click => {
                    let _ = MoveToEx(mem, x - 12, y, None);
                    let _ = LineTo(mem, x + 13, y);
                    let _ = MoveToEx(mem, x, y - 12, None);
                    let _ = LineTo(mem, x, y + 13);
                    text(mem, x + 10, y + 8, &m.text, color);
                }
                _ => {
                    let _ = Rectangle(mem, x, y, x + m.rect.w.max(2), y + m.rect.h.max(2));
                    let ty = if m.kind == MarkKind::Ocr {
                        y + m.rect.h + 1
                    } else {
                        y - 16
                    };
                    text(mem, x, ty, &m.text, color);
                }
            }
            SelectObject(mem, old_pen);
            let _ = DeleteObject(pen.into());
        }
        SelectObject(mem, old_brush);
        let _ = BitBlt(screen, 0, 0, vw, vh, Some(mem), 0, 0, SRCCOPY);
        SelectObject(mem, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mem);
        ReleaseDC(Some(hwnd), screen);
    }
}
