//! Windows side of the listening indicator: a small click-through window that never takes
//! the focus, drawn with per-pixel alpha (layered window) next to the text cursor of the
//! app in front. The cursor position comes from the app's GUI thread (GetGUIThreadInfo);
//! apps that don't report one get the bubble at the bottom center of their screen.
//!
//! The window exists only while the bubble is visible: a hidden window that nobody reads
//! messages for could make programs that broadcast messages wait.

use crate::indicator::{self, Mode};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{HWND, POINT, RECT, SIZE};
use windows_sys::Win32::Graphics::Gdi::{
    ClientToScreen, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, GetMonitorInfoW,
    MonitorFromWindow, ReleaseDC, SelectObject, AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{
    GetDpiForWindow, SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetForegroundWindow, GetGUIThreadInfo,
    GetWindowThreadProcessId, PeekMessageW, RegisterClassW, SetWindowPos, ShowWindow, TranslateMessage,
    UpdateLayeredWindow, GUITHREADINFO, HWND_TOPMOST, MSG, PM_REMOVE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SW_SHOWNOACTIVATE, ULW_ALPHA, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
};

const FOLLOW: Duration = Duration::from_millis(150);
const GAP: f32 = 8.0;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

struct Window {
    hwnd: HWND,
    dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u32,
    size: (i32, i32),
    scale: f32,
}

impl Drop for Window {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.hwnd);
            SelectObject(self.dc, self.old);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}

pub struct Overlay {
    win: Option<Window>,
    pos: (i32, i32),
    placed: Option<Instant>,
}

impl Overlay {
    pub fn open() -> Option<Self> {
        // Real pixels on every monitor (otherwise Windows scales our coordinates).
        unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        Some(Self { win: None, pos: (0, 0), placed: None })
    }

    pub fn frame(&mut self, mode: Mode, level: f32, t: f32) -> bool {
        let fg = unsafe { GetForegroundWindow() };
        let scale = if fg.is_null() { 1.0 } else { (unsafe { GetDpiForWindow(fg) } as f32 / 96.0).max(1.0) };
        if self.win.as_ref().is_some_and(|w| (w.scale - scale).abs() > 0.01) {
            self.win = None; // moved to a screen with another scale
        }
        if self.win.is_none() {
            match Window::new(scale) {
                Some(w) => self.win = Some(w),
                None => return false,
            }
            self.placed = None;
        }
        let Some(win) = &self.win else { return false };
        if self.placed.is_none_or(|p| p.elapsed() >= FOLLOW) {
            self.pos = place(fg, win.size, scale);
            self.placed = Some(Instant::now());
        }
        let (w, h) = win.size;
        let pixels = indicator::render(mode, level, t, scale);
        debug_assert_eq!(pixels.len(), (w * h) as usize);
        unsafe {
            std::ptr::copy_nonoverlapping(pixels.as_ptr(), win.bits, pixels.len());
            let screen = GetDC(std::ptr::null_mut());
            let dst = POINT { x: self.pos.0, y: self.pos.1 };
            let size = SIZE { cx: w, cy: h };
            let src = POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            UpdateLayeredWindow(win.hwnd, screen, &dst, &size, win.dc, &src, 0, &blend, ULW_ALPHA);
            ReleaseDC(std::ptr::null_mut(), screen);
            ShowWindow(win.hwnd, SW_SHOWNOACTIVATE);
            // Stay above windows that became topmost after us.
            SetWindowPos(win.hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            let mut msg: MSG = std::mem::zeroed();
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        true
    }

    pub fn hide(&mut self) {
        self.win = None;
    }
}

impl Window {
    fn new(scale: f32) -> Option<Self> {
        let (w, h) = indicator::render_size(scale);
        unsafe {
            let instance = GetModuleHandleW(std::ptr::null());
            let class = wide("WortflussIndicator");
            let wc = WNDCLASSW {
                lpfnWndProc: Some(DefWindowProcW),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                ..std::mem::zeroed()
            };
            RegisterClassW(&wc); // fails harmlessly once registered
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class.as_ptr(),
                class.as_ptr(),
                WS_POPUP,
                0,
                0,
                w,
                h,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance,
                std::ptr::null(),
            );
            if hwnd.is_null() {
                return None;
            }
            let dc = CreateCompatibleDC(std::ptr::null_mut());
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader = BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // top-down rows
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            };
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
            if bitmap.is_null() || bits.is_null() {
                DeleteDC(dc);
                DestroyWindow(hwnd);
                return None;
            }
            let old = SelectObject(dc, bitmap);
            Some(Self { hwnd, dc, bitmap, old, bits: bits as *mut u32, size: (w, h), scale })
        }
    }
}

/// Top-left corner for the bubble: right of the text cursor, else bottom center of the
/// screen the app is on.
fn place(fg: HWND, (w, h): (i32, i32), scale: f32) -> (i32, i32) {
    let work = work_area(fg);
    let (x, y) = match caret(fg) {
        Some(r) => (r.right + (GAP * scale) as i32, (r.top + r.bottom) / 2 - h / 2),
        None => match work {
            Some(a) => ((a.left + a.right) / 2 - w / 2, a.bottom - h - (96.0 * scale) as i32),
            None => (0, 0),
        },
    };
    match work {
        Some(a) => (x.clamp(a.left + 4, (a.right - w - 4).max(a.left)), y.clamp(a.top + 4, (a.bottom - h - 4).max(a.top))),
        None => (x, y),
    }
}

/// The text cursor of the app in front, in screen pixels.
fn caret(fg: HWND) -> Option<RECT> {
    if fg.is_null() {
        return None;
    }
    unsafe {
        let thread = GetWindowThreadProcessId(fg, std::ptr::null_mut());
        let mut gui = GUITHREADINFO { cbSize: std::mem::size_of::<GUITHREADINFO>() as u32, ..Default::default() };
        if GetGUIThreadInfo(thread, &mut gui) == 0 || gui.hwndCaret.is_null() {
            return None;
        }
        let r = gui.rcCaret;
        if r.bottom <= r.top {
            return None;
        }
        let mut p = POINT { x: r.left, y: r.top };
        if ClientToScreen(gui.hwndCaret, &mut p) == 0 {
            return None;
        }
        Some(RECT { left: p.x, top: p.y, right: p.x + (r.right - r.left), bottom: p.y + (r.bottom - r.top) })
    }
}

fn work_area(fg: HWND) -> Option<RECT> {
    unsafe {
        let monitor = MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..std::mem::zeroed() };
        (GetMonitorInfoW(monitor, &mut info) != 0).then_some(info.rcWork)
    }
}
