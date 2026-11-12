//! Native platform boundary.  The Windows implementation deliberately uses only
//! documented user32/GDI APIs and never reads process or game memory.

use crate::{Backend, Result, model};

#[cfg(not(windows))]
pub struct WindowsBackend;

#[cfg(not(windows))]
impl WindowsBackend {
    pub fn new(_: model::Target) -> Result<Self> {
        Err("Windows backend is unavailable on this platform".into())
    }
}

#[cfg(not(windows))]
impl Backend for WindowsBackend {
    fn focused(&mut self) -> Result<bool> {
        Err("Windows backend is unavailable on this platform".into())
    }
    fn key_down(&mut self, _: u16) -> Result<bool> {
        Err("Windows backend is unavailable on this platform".into())
    }
    fn cursor(&mut self) -> Result<model::Point> {
        Err("Windows backend is unavailable on this platform".into())
    }
    fn pixel(&mut self, _: model::Point) -> Result<u32> {
        Err("Windows backend is unavailable on this platform".into())
    }
    fn clipboard(&mut self) -> Result<String> {
        Err("Windows backend is unavailable on this platform".into())
    }
    fn emit(&mut self, _: &model::InputEvent) -> Result<()> {
        Err("Windows backend is unavailable on this platform".into())
    }
    fn screenshot(&mut self, _: Option<model::Region>) -> Result<model::Capture> {
        Err("Windows backend is unavailable on this platform".into())
    }
    fn ocr(&mut self, _: Option<model::Region>) -> Result<Vec<model::TextBox>> {
        Err("Windows OCR is unavailable on this platform".into())
    }
    fn notify(&mut self, _: &str, _: u64) -> Result<()> {
        Err("Windows notifications are unavailable on this platform".into())
    }
}

#[cfg(windows)]
mod windows_backend {
    use super::*;
    use std::{
        cell::Cell,
        future::IntoFuture,
        mem::{size_of, zeroed},
        sync::Arc,
        task::{Context, Poll, Wake, Waker},
        thread,
        time::{Duration, Instant},
    };
    use windows::{
        Win32::{
            Foundation::{HWND, LPARAM, POINT, RECT},
            Graphics::Gdi::{
                BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, ClientToScreen,
                CreateCompatibleBitmap, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject,
                GetDC, GetDIBits, GetPixel, HGDIOBJ, ReleaseDC, SRCCOPY, ScreenToClient,
                SelectObject,
            },
            UI::{
                Input::KeyboardAndMouse::{
                    GetAsyncKeyState, INPUT, INPUT_0, INPUT_TYPE, KEYBDINPUT, KEYEVENTF_KEYUP,
                    MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
                    MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE,
                    MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK,
                    MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, MOUSEINPUT, SendInput, VIRTUAL_KEY,
                },
                WindowsAndMessaging::{
                    EnumWindows, GetClassNameW, GetClientRect, GetCursorPos, GetForegroundWindow,
                    GetSystemMetrics, GetWindowTextLengthW, GetWindowTextW, IsWindowVisible,
                    SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
                },
            },
        },
        core::BOOL,
    };

    #[derive(Clone)]
    pub struct WindowsBackend {
        target: model::Target,
        hwnd: Cell<HWND>,
    }

    struct ThreadWaker(thread::Thread);
    impl Wake for ThreadWaker {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
        fn wake_by_ref(self: &Arc<Self>) {
            self.0.unpark();
        }
    }
    fn block_winrt<F: std::future::Future>(future: F) -> Result<F::Output> {
        let mut future = Box::pin(future);
        let waker = Waker::from(Arc::new(ThreadWaker(thread::current())));
        let mut cx = Context::from_waker(&waker);
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            match future.as_mut().poll(&mut cx) {
                Poll::Ready(v) => return Ok(v),
                Poll::Pending if Instant::now() >= deadline => return Err("OCR timed out".into()),
                Poll::Pending => thread::park_timeout(Duration::from_millis(10)),
            }
        }
    }

    impl WindowsBackend {
        pub fn new(target: model::Target) -> Result<Self> {
            if target.window_class.is_none() && target.window_title.is_none() {
                return Err("a window selector is required".into());
            }
            for value in [
                target.window_class.as_deref(),
                target.window_title.as_deref(),
            ]
            .into_iter()
            .flatten()
            {
                if value.trim().is_empty() || value.contains('*') || value.contains('?') {
                    return Err("window selectors must be nonempty exact strings".into());
                }
            }
            Ok(Self {
                target,
                hwnd: Cell::new(HWND::default()),
            })
        }

        fn refresh(&self) -> Result<bool> {
            let mut found = HWND::default();
            unsafe {
                EnumWindows(
                    Some(find_window),
                    LPARAM(&mut (&mut found, &self.target) as *mut _ as isize),
                )
                .map_err(|e| format!("EnumWindows: {e}"))?;
            }
            self.hwnd.set(found);
            Ok(!found.0.is_null())
        }

        fn check_window(&self) -> Result<()> {
            if self.refresh()? {
                Ok(())
            } else {
                Err("target window is unavailable (backend dormant)".into())
            }
        }
        fn client_size(&self) -> Result<(i32, i32)> {
            self.check_window()?;
            let mut r = RECT::default();
            unsafe {
                GetClientRect(self.hwnd.get(), &mut r)
                    .map_err(|e| format!("GetClientRect: {e}"))?;
            }
            let (w, h) = (r.right - r.left, r.bottom - r.top);
            if w <= 0 || h <= 0 {
                Err("target client area is empty".into())
            } else {
                Ok((w, h))
            }
        }
        fn client_point(&self, p: model::Point) -> Result<POINT> {
            let (w, h) = self.client_size()?;
            if !p.x.is_finite()
                || !p.y.is_finite()
                || !(0.0..=1.0).contains(&p.x)
                || !(0.0..=1.0).contains(&p.y)
            {
                return Err("point must be normalized to 0..1".into());
            }
            let mut q = POINT {
                x: (p.x * (w - 1) as f64).round() as i32,
                y: (p.y * (h - 1) as f64).round() as i32,
            };
            unsafe {
                ClientToScreen(self.hwnd.get(), &mut q)
                    .ok()
                    .map_err(|e| format!("ClientToScreen: {e}"))?;
            }
            Ok(q)
        }
        fn region(&self, r: Option<model::Region>) -> Result<(i32, i32, i32, i32)> {
            let (w, h) = self.client_size()?;
            let r = r.unwrap_or(model::Region {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            });
            if ![r.x, r.y, r.width, r.height].iter().all(|v| v.is_finite())
                || r.x < 0.0
                || r.y < 0.0
                || r.width <= 0.0
                || r.height <= 0.0
                || r.x + r.width > 1.0
                || r.y + r.height > 1.0
            {
                return Err("region must be within normalized client bounds".into());
            }
            let x = (r.x * w as f64).round() as i32;
            let y = (r.y * h as f64).round() as i32;
            let rw = (r.width * w as f64).round() as i32;
            let rh = (r.height * h as f64).round() as i32;
            if rw <= 0 || rh <= 0 || x >= w || y >= h {
                Err("region is empty".into())
            } else {
                Ok((x, y, rw.min(w - x), rh.min(h - y)))
            }
        }
        fn send_mouse(&self, p: model::Point, button: Option<(u8, bool)>) -> Result<()> {
            let (dx, dy, flags, data) = if let Some((button, down)) = button {
                let (on, off, data) = match button {
                    1 => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, 0),
                    2 => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, 0),
                    3 => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, 0),
                    4 => (MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, 1),
                    5 => (MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, 2),
                    _ => return Err("mouse button must be 1 through 5".into()),
                };
                (0, 0, if down { on } else { off }, data)
            } else {
                let q = self.client_point(p)?;
                let (x, y, w, h) = unsafe {
                    (
                        GetSystemMetrics(SM_XVIRTUALSCREEN),
                        GetSystemMetrics(SM_YVIRTUALSCREEN),
                        GetSystemMetrics(SM_CXVIRTUALSCREEN),
                        GetSystemMetrics(SM_CYVIRTUALSCREEN),
                    )
                };
                if w <= 1 || h <= 1 {
                    return Err("virtual desktop is empty".into());
                }
                let scale = |v: i32, origin: i32, extent: i32| {
                    (((i64::from(v) - i64::from(origin)) * 65535) / i64::from(extent - 1))
                        .clamp(0, 65535) as i32
                };
                (
                    scale(q.x, x, w),
                    scale(q.y, y, h),
                    MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                    0,
                )
            };
            let mi = MOUSEINPUT {
                dx,
                dy,
                mouseData: data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            };
            let input = INPUT {
                r#type: INPUT_TYPE(0),
                Anonymous: INPUT_0 { mi },
            };
            unsafe {
                if SendInput(&[input], size_of::<INPUT>() as i32) != 1 {
                    return Err("SendInput failed".into());
                }
            }
            Ok(())
        }
        fn capture(&self, region: Option<model::Region>) -> Result<model::Capture> {
            let (x, y, w, h) = self.region(region)?;
            let len = (w as usize)
                .checked_mul(h as usize)
                .and_then(|n| n.checked_mul(4))
                .filter(|&n| n <= 256 * 1024 * 1024)
                .ok_or("capture is too large")?;
            let hwnd = self.hwnd.get();
            let image = self.capture_dc(Some(hwnd), x, y, w, h, len)?;
            if image
                .bgra
                .chunks_exact(4)
                .any(|pixel| pixel[..3] != [0, 0, 0])
            {
                return Ok(image);
            }
            unsafe {
                if GetForegroundWindow() != hwnd {
                    return Err("capture is black and target is not foreground".into());
                }
                let mut origin = POINT { x, y };
                ClientToScreen(hwnd, &mut origin)
                    .ok()
                    .map_err(|e| format!("ClientToScreen: {e}"))?;
                let (left, top, width, height) = (
                    GetSystemMetrics(SM_XVIRTUALSCREEN),
                    GetSystemMetrics(SM_YVIRTUALSCREEN),
                    GetSystemMetrics(SM_CXVIRTUALSCREEN),
                    GetSystemMetrics(SM_CYVIRTUALSCREEN),
                );
                if origin.x < left
                    || origin.y < top
                    || i64::from(origin.x) + i64::from(w) > i64::from(left) + i64::from(width)
                    || i64::from(origin.y) + i64::from(h) > i64::from(top) + i64::from(height)
                {
                    return Err("capture region extends outside the desktop".into());
                }
                let image = self.capture_dc(None, origin.x, origin.y, w, h, len)?;
                if GetForegroundWindow() != hwnd {
                    return Err("target lost focus during capture".into());
                }
                if image
                    .bgra
                    .chunks_exact(4)
                    .all(|pixel| pixel[..3] == [0, 0, 0])
                {
                    return Err("capture is entirely black".into());
                }
                Ok(image)
            }
        }
        fn capture_dc(
            &self,
            hwnd: Option<HWND>,
            x: i32,
            y: i32,
            w: i32,
            h: i32,
            len: usize,
        ) -> Result<model::Capture> {
            unsafe {
                let dc = GetDC(hwnd);
                if dc.0.is_null() {
                    return Err("GetDC failed".into());
                }
                let mem = CreateCompatibleDC(Some(dc));
                let bmp = CreateCompatibleBitmap(dc, w, h);
                if mem.0.is_null() || bmp.0.is_null() {
                    if !bmp.0.is_null() {
                        let _ = DeleteObject(HGDIOBJ(bmp.0));
                    }
                    if !mem.0.is_null() {
                        let _ = DeleteDC(mem);
                    }
                    let _ = ReleaseDC(hwnd, dc);
                    return Err("capture surface failed".into());
                }
                let old = SelectObject(mem, HGDIOBJ(bmp.0));
                let ok =
                    !old.0.is_null() && BitBlt(mem, 0, 0, w, h, Some(dc), x, y, SRCCOPY).is_ok();
                if !old.0.is_null() {
                    let _ = SelectObject(mem, old);
                }
                let mut bi: BITMAPINFO = zeroed();
                bi.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
                bi.bmiHeader.biWidth = w;
                bi.bmiHeader.biHeight = -h;
                bi.bmiHeader.biPlanes = 1;
                bi.bmiHeader.biBitCount = 32;
                bi.bmiHeader.biCompression = BI_RGB.0;
                let mut bgra = vec![0; len];
                let got = if ok {
                    GetDIBits(
                        mem,
                        bmp,
                        0,
                        h as u32,
                        Some(bgra.as_mut_ptr().cast()),
                        &mut bi,
                        DIB_RGB_COLORS,
                    )
                } else {
                    0
                };
                let _ = DeleteObject(HGDIOBJ(bmp.0));
                let _ = DeleteDC(mem);
                let _ = ReleaseDC(hwnd, dc);
                if !ok || got != h {
                    return Err("capture failed".into());
                }
                Ok(model::Capture {
                    width: w as u32,
                    height: h as u32,
                    bgra,
                })
            }
        }
    }

    unsafe extern "system" fn find_window(hwnd: HWND, data: LPARAM) -> BOOL {
        unsafe {
            let pair = &mut *(data.0 as *mut (&mut HWND, &model::Target));
            if !pair.0.0.is_null() {
                return true.into();
            }
            if !IsWindowVisible(hwnd).as_bool() {
                return true.into();
            }
            let mut class = [0u16; 256];
            let n = GetClassNameW(hwnd, &mut class) as usize;
            let cls = String::from_utf16_lossy(&class[..n]);
            let len = GetWindowTextLengthW(hwnd);
            let mut title = vec![0u16; (len.max(0) + 1) as usize];
            let n = GetWindowTextW(hwnd, &mut title) as usize;
            let title = String::from_utf16_lossy(&title[..n]);
            if pair.1.window_class.as_ref().is_some_and(|v| v != &cls)
                || pair.1.window_title.as_ref().is_some_and(|v| v != &title)
            {
                return true.into();
            }
            *pair.0 = hwnd;
            true.into()
        }
    }

    impl Backend for WindowsBackend {
        fn sound(&mut self, cue: model::SoundCue) -> Result<()> {
            use windows::Win32::{
                System::Diagnostics::Debug::MessageBeep,
                UI::WindowsAndMessaging::{
                    MB_ICONERROR, MB_ICONINFORMATION, MB_ICONWARNING, MB_OK,
                },
            };
            let kind = match cue {
                model::SoundCue::Default => MB_OK,
                model::SoundCue::Info => MB_ICONINFORMATION,
                model::SoundCue::Warning => MB_ICONWARNING,
                model::SoundCue::Error => MB_ICONERROR,
            };
            unsafe { MessageBeep(kind) }.map_err(|e| e.to_string())
        }
        fn target_present(&mut self) -> Result<bool> {
            self.refresh()
        }
        fn focused(&mut self) -> Result<bool> {
            Ok(self.refresh()? && unsafe { GetForegroundWindow() == self.hwnd.get() })
        }
        fn key_down(&mut self, key: u16) -> Result<bool> {
            if key == 0 {
                return Err("virtual key must be nonzero".into());
            };
            Ok(unsafe { GetAsyncKeyState(key as i32) < 0 })
        }
        fn cursor(&mut self) -> Result<model::Point> {
            let (w, h) = self.client_size()?;
            let mut p = POINT::default();
            unsafe {
                GetCursorPos(&mut p).map_err(|e| format!("GetCursorPos: {e}"))?;
                if !ScreenToClient(self.hwnd.get(), &mut p).as_bool() {
                    return Err("ScreenToClient failed".into());
                }
            }
            if p.x < 0 || p.y < 0 || p.x >= w || p.y >= h {
                return Err("cursor is outside target client area".into());
            }
            Ok(model::Point {
                x: p.x as f64 / (w - 1).max(1) as f64,
                y: p.y as f64 / (h - 1).max(1) as f64,
            })
        }
        fn pixel(&mut self, p: model::Point) -> Result<u32> {
            let q = self.client_point(p)?;
            unsafe {
                let dc = GetDC(None);
                if dc.0.is_null() {
                    return Err("GetDC failed".into());
                };
                let c = GetPixel(dc, q.x, q.y);
                ReleaseDC(None, dc);
                if c.0 == 0xffff_ffff {
                    return Err("GetPixel failed".into());
                }
                // COLORREF stores BGR; the public profile contract uses RRGGBB.
                let bgr = c.0;
                Ok(((bgr & 0xff) << 16) | (bgr & 0xff00) | ((bgr >> 16) & 0xff))
            }
        }
        fn clipboard(&mut self) -> Result<String> {
            unsafe {
                windows::Win32::System::DataExchange::OpenClipboard(None)
                    .map_err(|e| format!("OpenClipboard: {e}"))?;
                let h = match windows::Win32::System::DataExchange::GetClipboardData(13) {
                    Ok(h) => h,
                    Err(e) => {
                        let _ = windows::Win32::System::DataExchange::CloseClipboard();
                        return Err(format!("GetClipboardData: {e}"));
                    }
                };
                let size = windows::Win32::System::Memory::GlobalSize(
                    windows::Win32::Foundation::HGLOBAL(h.0),
                );
                if size == 0 || size > 16 * 1024 * 1024 {
                    let _ = windows::Win32::System::DataExchange::CloseClipboard();
                    return Err("clipboard data is invalid or too large".into());
                };
                let p = windows::Win32::System::Memory::GlobalLock(
                    windows::Win32::Foundation::HGLOBAL(h.0),
                );
                if p.is_null() {
                    let _ = windows::Win32::System::DataExchange::CloseClipboard();
                    return Err("GlobalLock failed".into());
                };
                let s = String::from_utf16_lossy(
                    std::slice::from_raw_parts(p as *const u16, size / 2)
                        .split(|v| *v == 0)
                        .next()
                        .unwrap_or(&[]),
                );
                let _ = windows::Win32::System::Memory::GlobalUnlock(
                    windows::Win32::Foundation::HGLOBAL(h.0),
                );
                let _ = windows::Win32::System::DataExchange::CloseClipboard();
                Ok(s)
            }
        }
        fn emit(&mut self, e: &model::InputEvent) -> Result<()> {
            match e {
                model::InputEvent::Wheel { delta, horizontal } => {
                    use windows::Win32::UI::Input::KeyboardAndMouse::{
                        MOUSEEVENTF_HWHEEL, MOUSEEVENTF_WHEEL,
                    };
                    let input = INPUT {
                        r#type: INPUT_TYPE(0),
                        Anonymous: INPUT_0 {
                            mi: MOUSEINPUT {
                                mouseData: *delta as u32,
                                dwFlags: if *horizontal {
                                    MOUSEEVENTF_HWHEEL
                                } else {
                                    MOUSEEVENTF_WHEEL
                                },
                                ..Default::default()
                            },
                        },
                    };
                    if unsafe { SendInput(&[input], size_of::<INPUT>() as i32) } != 1 {
                        return Err("SendInput failed".into());
                    }
                    Ok(())
                }
                model::InputEvent::Key { key, down } => {
                    if *key == 0 {
                        return Err("virtual key must be nonzero".into());
                    };
                    let k = KEYBDINPUT {
                        wVk: VIRTUAL_KEY(*key),
                        wScan: 0,
                        dwFlags: if *down {
                            Default::default()
                        } else {
                            KEYEVENTF_KEYUP
                        },
                        time: 0,
                        dwExtraInfo: 0,
                    };
                    let i = INPUT {
                        r#type: INPUT_TYPE(1),
                        Anonymous: INPUT_0 { ki: k },
                    };
                    unsafe {
                        if SendInput(&[i], size_of::<INPUT>() as i32) != 1 {
                            return Err("SendInput failed".into());
                        }
                    };
                    Ok(())
                }
                model::InputEvent::Move { x, y } => {
                    self.send_mouse(model::Point { x: *x, y: *y }, None)
                }
                model::InputEvent::Button { button, down } => {
                    self.send_mouse(model::Point { x: 0.5, y: 0.5 }, Some((*button, *down)))
                }
                model::InputEvent::Text { text } => {
                    for c in text.encode_utf16() {
                        let a = KEYBDINPUT {
                            wVk: VIRTUAL_KEY(0),
                            wScan: c,
                            dwFlags: windows::Win32::UI::Input::KeyboardAndMouse::KEYEVENTF_UNICODE,
                            time: 0,
                            dwExtraInfo: 0,
                        };
                        let b = KEYBDINPUT {
                            dwFlags: windows::Win32::UI::Input::KeyboardAndMouse::KEYEVENTF_UNICODE
                                | KEYEVENTF_KEYUP,
                            ..a
                        };
                        let v = [
                            INPUT {
                                r#type: INPUT_TYPE(1),
                                Anonymous: INPUT_0 { ki: a },
                            },
                            INPUT {
                                r#type: INPUT_TYPE(1),
                                Anonymous: INPUT_0 { ki: b },
                            },
                        ];
                        unsafe {
                            if SendInput(&v, size_of::<INPUT>() as i32) != 2 {
                                return Err("SendInput failed".into());
                            }
                        }
                    }
                    Ok(())
                }
            }
        }
        fn screenshot(&mut self, r: Option<model::Region>) -> Result<model::Capture> {
            self.capture(r)
        }
        fn ocr(&mut self, region: Option<model::Region>) -> Result<Vec<model::TextBox>> {
            let cap = self.capture(region)?;
            let (cw, ch) = self.client_size()?;
            let writer = windows::Storage::Streams::DataWriter::new()
                .map_err(|e| format!("DataWriter: {e}"))?;
            writer
                .WriteBytes(&cap.bgra)
                .map_err(|e| format!("WriteBytes: {e}"))?;
            let buffer = writer
                .DetachBuffer()
                .map_err(|e| format!("DetachBuffer: {e}"))?;
            let bitmap = windows::Graphics::Imaging::SoftwareBitmap::CreateCopyFromBuffer(
                &buffer,
                windows::Graphics::Imaging::BitmapPixelFormat::Bgra8,
                cap.width as i32,
                cap.height as i32,
            )
            .map_err(|e| format!("SoftwareBitmap: {e}"))?;
            let engine = windows::Media::Ocr::OcrEngine::TryCreateFromUserProfileLanguages()
                .map_err(|e| format!("OcrEngine: {e}"))?;
            let operation = engine
                .RecognizeAsync(&bitmap)
                .map_err(|e| format!("RecognizeAsync: {e}"))?;
            let result = match block_winrt(operation.clone().into_future()) {
                Ok(result) => result.map_err(|e| format!("OCR: {e}"))?,
                Err(e) => {
                    let _ = operation.Cancel();
                    return Err(e);
                }
            };
            let (ox, oy, _, _) = self.region(region)?;
            let lines = result.Lines().map_err(|e| format!("OCR lines: {e}"))?;
            let mut out = Vec::new();
            for i in 0..lines.Size().map_err(|e| format!("OCR size: {e}"))? {
                let line = lines.GetAt(i).map_err(|e| format!("OCR line: {e}"))?;
                let words = line.Words().map_err(|e| format!("OCR words: {e}"))?;
                for j in 0..words.Size().map_err(|e| format!("OCR word count: {e}"))? {
                    let word = words.GetAt(j).map_err(|e| format!("OCR word: {e}"))?;
                    let r = word
                        .BoundingRect()
                        .map_err(|e| format!("OCR bounds: {e}"))?;
                    out.push(model::TextBox {
                        text: word
                            .Text()
                            .map_err(|e| format!("OCR word text: {e}"))?
                            .to_string(),
                        region: model::Region {
                            x: (r.X as f64 + ox as f64) / (cw as f64),
                            y: (r.Y as f64 + oy as f64) / (ch as f64),
                            width: r.Width as f64 / (cw as f64),
                            height: r.Height as f64 / (ch as f64),
                        },
                    });
                }
            }
            Ok(out)
        }
        fn notify(&mut self, message: &str, _ttl_ms: u64) -> Result<()> {
            if message.is_empty() {
                return Err("notification message is empty".into());
            }
            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::ffi::c_void;
        #[link(name = "user32")]
        unsafe extern "system" {
            fn CreateWindowExW(
                ex: u32,
                class: *const u16,
                title: *const u16,
                style: u32,
                x: i32,
                y: i32,
                w: i32,
                h: i32,
                parent: *mut c_void,
                menu: *mut c_void,
                instance: *mut c_void,
                param: *mut c_void,
            ) -> *mut c_void;
            fn DestroyWindow(hwnd: *mut c_void) -> i32;
        }
        #[link(name = "gdi32")]
        unsafe extern "system" {
            fn SetPixel(dc: *mut c_void, x: i32, y: i32, color: u32) -> u32;
        }
        fn wide(s: &str) -> Vec<u16> {
            s.encode_utf16().chain(Some(0)).collect()
        }
        struct TestWindow(*mut c_void);
        impl Drop for TestWindow {
            fn drop(&mut self) {
                unsafe {
                    DestroyWindow(self.0);
                }
            }
        }
        fn window(title: &str) -> TestWindow {
            let hwnd = unsafe {
                CreateWindowExW(
                    0x08000000,
                    wide("STATIC").as_ptr(),
                    wide(title).as_ptr(),
                    0x90000000,
                    0,
                    0,
                    64,
                    64,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            assert!(!hwnd.is_null());
            TestWindow(hwnd)
        }
        #[test]
        fn selector_and_window_lifecycle_capture() {
            assert!(
                WindowsBackend::new(model::Target {
                    window_class: None,
                    window_title: None
                })
                .is_err()
            );
            assert!(
                WindowsBackend::new(model::Target {
                    window_class: Some("*".into()),
                    window_title: None
                })
                .is_err()
            );
            let title = format!("RHK platform test {}", std::process::id());
            let mut backend = WindowsBackend::new(model::Target {
                window_class: Some("Static".into()),
                window_title: Some(title.clone()),
            })
            .unwrap();
            assert!(!backend.target_present().unwrap());
            assert!(!backend.focused().unwrap());
            let first = window(&title);
            assert!(backend.target_present().unwrap());
            unsafe {
                let dc = GetDC(Some(HWND(first.0)));
                assert!(!dc.0.is_null());
                assert_ne!(SetPixel(dc.0, 2, 2, 0x00030201), u32::MAX);
                assert_ne!(SetPixel(dc.0, 3, 3, 0), u32::MAX);
                let _ = ReleaseDC(Some(HWND(first.0)), dc);
            }
            if unsafe { GetForegroundWindow() } != HWND(first.0) {
                let black = model::Region {
                    x: 3.0 / 64.0,
                    y: 3.0 / 64.0,
                    width: 1.0 / 64.0,
                    height: 1.0 / 64.0,
                };
                assert!(backend.screenshot(Some(black)).is_err());
            }
            let image = backend.screenshot(None).unwrap();
            assert_eq!(
                backend
                    .pixel(model::Point {
                        x: 2.0 / 64.0,
                        y: 2.0 / 64.0
                    })
                    .unwrap(),
                0x010203
            );
            assert_eq!(
                image.bgra.len(),
                image.width as usize * image.height as usize * 4
            );
            let i = (2 * image.width as usize + 2) * 4;
            assert_eq!(&image.bgra[i..i + 3], &[3, 2, 1]);
            assert!(
                backend
                    .screenshot(Some(model::Region {
                        x: 0.9,
                        y: 0.0,
                        width: 0.2,
                        height: 1.0
                    }))
                    .is_err()
            );
            drop(first);
            assert!(!backend.target_present().unwrap());
            let second = window(&title);
            assert!(backend.target_present().unwrap());
            drop(second);
            assert!(!backend.target_present().unwrap());
        }
    }
}

#[cfg(windows)]
pub use windows_backend::WindowsBackend;
