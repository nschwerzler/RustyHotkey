//! Caller-controlled, client-relative annotations. No process inspection or hooks.
use crate::{
    Result,
    model::{Point, Region, Target},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlaySpec {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub elements: Vec<OverlayElement>,
    #[serde(default)]
    pub ttl_ms: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum OverlayElement {
    Box {
        region: Region,
        color: u32,
        #[serde(default)]
        fill_color: Option<u32>,
        #[serde(default)]
        stroke_width: Option<u32>,
        #[serde(default)]
        border_radius: Option<u32>,
    },
    Circle {
        region: Region,
        color: u32,
    },
    Label {
        point: Point,
        text: String,
        color: u32,
        #[serde(default)]
        font_size: Option<u32>,
        #[serde(default)]
        font_family: Option<String>,
        #[serde(default)]
        max_width: Option<u32>,
        #[serde(default)]
        shadow: Option<bool>,
    },
    Point {
        point: Point,
        color: u32,
    },
}

impl OverlaySpec {
    pub fn validate(&self) -> Result<()> {
        if self.elements.len() > 4096 {
            return Err("too many overlay elements".into());
        }
        if self
            .status
            .as_ref()
            .is_some_and(|s| s.len() > 4096 || s.contains('\0'))
        {
            return Err("invalid overlay status".into());
        }
        for e in &self.elements {
            let (p, region, color) = match e {
                OverlayElement::Box {
                    region,
                    color,
                    fill_color,
                    stroke_width,
                    border_radius,
                } => {
                    if fill_color.is_some_and(|c| c == 0 || c > 0xffffff)
                        || stroke_width.is_some_and(|v| v > 8)
                        || border_radius.is_some_and(|v| v > 32)
                    {
                        return Err("invalid overlay box style".into());
                    }
                    (None, Some(region), color)
                }
                OverlayElement::Circle { region, color } => (None, Some(region), color),
                OverlayElement::Label {
                    point,
                    text,
                    color,
                    font_size,
                    font_family,
                    max_width,
                    ..
                } => {
                    if text.len() > 4096 || text.contains('\0') {
                        return Err("invalid overlay label".into());
                    }
                    if font_size.is_some_and(|v| !(8..=48).contains(&v))
                        || max_width.is_some_and(|v| !(16..=1024).contains(&v))
                        || font_family
                            .as_ref()
                            .is_some_and(|v| v.is_empty() || v.len() > 64 || v.contains('\0'))
                    {
                        return Err("invalid overlay label style".into());
                    }
                    (Some(point), None, color)
                }
                OverlayElement::Point { point, color } => (Some(point), None, color),
            };
            if *color == 0 || *color > 0xffffff {
                return Err("overlay color must be a nonzero 0xRRGGBB value".into());
            }
            if let Some(p) = p {
                if !p.x.is_finite()
                    || !p.y.is_finite()
                    || !(0.0..=1.0).contains(&p.x)
                    || !(0.0..=1.0).contains(&p.y)
                {
                    return Err("overlay point must be normalized to 0..1".into());
                }
            }
            if let Some(r) = region {
                if ![r.x, r.y, r.width, r.height].iter().all(|v| v.is_finite())
                    || r.x < 0.0
                    || r.y < 0.0
                    || r.width <= 0.0
                    || r.height <= 0.0
                    || r.x + r.width > 1.0
                    || r.y + r.height > 1.0
                {
                    return Err("overlay region must be within normalized client bounds".into());
                }
            }
        }
        Ok(())
    }
}

#[cfg(not(windows))]
pub fn annotate_capture(
    _: crate::model::Capture,
    _: &OverlaySpec,
) -> Result<crate::model::Capture> {
    Err("overlay annotation is unavailable on this platform".into())
}

#[cfg(not(windows))]
pub struct Overlay;
#[cfg(not(windows))]
impl Overlay {
    pub fn new(_: Target) -> Result<Self> {
        Err("overlay is unavailable on this platform".into())
    }
    pub fn show(&mut self, _: OverlaySpec) -> Result<()> {
        Err("overlay is unavailable on this platform".into())
    }
    pub fn clear(&mut self) -> Result<()> {
        Err("overlay is unavailable on this platform".into())
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    use std::{
        ffi::c_void,
        mem::zeroed,
        ptr::{null, null_mut},
        sync::mpsc,
        thread::{self, JoinHandle},
        time::{Duration, Instant},
    };

    type Handle = *mut c_void;

    /// Paint an overlay onto an in-memory capture without creating a window.
    pub fn annotate_capture(
        mut capture: crate::model::Capture,
        spec: &OverlaySpec,
    ) -> Result<crate::model::Capture> {
        use windows::Win32::Graphics::Gdi::{
            BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection,
            DIB_RGB_COLORS, DeleteDC, DeleteObject as DeleteGdiObject, GdiFlush, HGDIOBJ,
        };
        spec.validate()?;
        let (w, h) = (
            i32::try_from(capture.width).map_err(|_| "invalid capture width")?,
            i32::try_from(capture.height).map_err(|_| "invalid capture height")?,
        );
        let len = (capture.width as usize)
            .checked_mul(capture.height as usize)
            .and_then(|n| n.checked_mul(4))
            .filter(|&n| n > 0 && n <= 256 * 1024 * 1024)
            .ok_or("invalid capture size")?;
        if w <= 0 || h <= 0 || capture.bgra.len() != len {
            return Err("invalid capture dimensions or pixels".into());
        }
        unsafe {
            let dc = CreateCompatibleDC(None);
            if dc.0.is_null() {
                return Err("annotation DC creation failed".into());
            }
            let mut info = BITMAPINFO::default();
            info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = w;
            info.bmiHeader.biHeight = -h;
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            info.bmiHeader.biCompression = BI_RGB.0;
            let mut bits = null_mut();
            let bitmap = match CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0) {
                Ok(bitmap) => bitmap,
                Err(e) => {
                    let _ = DeleteDC(dc);
                    return Err(format!("annotation bitmap creation: {e}").into());
                }
            };
            let old = SelectObject(dc.0, bitmap.0);
            if old.is_null() || old as isize == -1 || bits.is_null() {
                if !old.is_null() && old as isize != -1 {
                    SelectObject(dc.0, old);
                }
                let _ = DeleteGdiObject(HGDIOBJ(bitmap.0));
                let _ = DeleteDC(dc);
                return Err("annotation surface selection failed".into());
            }
            std::ptr::copy_nonoverlapping(capture.bgra.as_ptr(), bits.cast::<u8>(), len);
            paint(
                dc.0,
                Rect {
                    left: 0,
                    top: 0,
                    right: w,
                    bottom: h,
                },
                spec,
            );
            let flushed = GdiFlush().as_bool();
            if flushed {
                std::ptr::copy_nonoverlapping(bits.cast::<u8>(), capture.bgra.as_mut_ptr(), len);
            }
            SelectObject(dc.0, old);
            let _ = DeleteGdiObject(HGDIOBJ(bitmap.0));
            let _ = DeleteDC(dc);
            if !flushed {
                return Err("annotation drawing failed".into());
            }
        }
        Ok(capture)
    }
    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }
    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct Pt {
        x: i32,
        y: i32,
    }
    #[repr(C)]
    struct Msg {
        hwnd: Handle,
        message: u32,
        wparam: usize,
        lparam: isize,
        time: u32,
        point: Pt,
        private: u32,
    }
    #[repr(C)]
    struct Paint {
        dc: Handle,
        erase: i32,
        rect: Rect,
        restore: i32,
        inc_update: i32,
        reserved: [u8; 32],
    }
    #[repr(C)]
    struct WndClass {
        style: u32,
        proc: Option<unsafe extern "system" fn(Handle, u32, usize, isize) -> isize>,
        extra_class: i32,
        extra_window: i32,
        instance: Handle,
        icon: Handle,
        cursor: Handle,
        background: Handle,
        menu: *const u16,
        class: *const u16,
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn EnumWindows(
            cb: Option<unsafe extern "system" fn(Handle, isize) -> i32>,
            data: isize,
        ) -> i32;
        fn IsWindowVisible(w: Handle) -> i32;
        fn IsWindow(w: Handle) -> i32;
        fn GetForegroundWindow() -> Handle;
        fn GetClassNameW(w: Handle, s: *mut u16, n: i32) -> i32;
        fn GetWindowTextW(w: Handle, s: *mut u16, n: i32) -> i32;
        fn GetWindowTextLengthW(w: Handle) -> i32;
        fn RegisterClassW(c: *const WndClass) -> u16;
        fn CreateWindowExW(
            ex: u32,
            class: *const u16,
            title: *const u16,
            style: u32,
            x: i32,
            y: i32,
            w: i32,
            h: i32,
            parent: Handle,
            menu: Handle,
            instance: Handle,
            param: Handle,
        ) -> Handle;
        fn DestroyWindow(w: Handle) -> i32;
        fn DefWindowProcW(w: Handle, m: u32, a: usize, b: isize) -> isize;
        fn SetWindowLongPtrW(w: Handle, index: i32, value: isize) -> isize;
        fn GetWindowLongPtrW(w: Handle, index: i32) -> isize;
        fn SetLayeredWindowAttributes(w: Handle, key: u32, alpha: u8, flags: u32) -> i32;
        fn SetWindowDisplayAffinity(w: Handle, affinity: u32) -> i32;
        fn GetClientRect(w: Handle, r: *mut Rect) -> i32;
        fn ClientToScreen(w: Handle, p: *mut Pt) -> i32;
        fn SetWindowPos(
            w: Handle,
            after: Handle,
            x: i32,
            y: i32,
            cx: i32,
            cy: i32,
            flags: u32,
        ) -> i32;
        fn ShowWindow(w: Handle, cmd: i32) -> i32;
        fn PeekMessageW(m: *mut Msg, w: Handle, min: u32, max: u32, remove: u32) -> i32;
        fn TranslateMessage(m: *const Msg) -> i32;
        fn DispatchMessageW(m: *const Msg) -> isize;
        fn InvalidateRect(w: Handle, r: *const Rect, erase: i32) -> i32;
        fn BeginPaint(w: Handle, p: *mut Paint) -> Handle;
        fn EndPaint(w: Handle, p: *const Paint) -> i32;
        fn FillRect(dc: Handle, r: *const Rect, brush: Handle) -> i32;
        fn DrawTextW(dc: Handle, text: *const u16, count: i32, r: *mut Rect, format: u32) -> i32;
    }
    #[link(name = "gdi32")]
    unsafe extern "system" {
        fn CreateSolidBrush(color: u32) -> Handle;
        fn CreatePen(style: i32, width: i32, color: u32) -> Handle;
        fn SelectObject(dc: Handle, obj: Handle) -> Handle;
        fn DeleteObject(obj: Handle) -> i32;
        fn GetStockObject(index: i32) -> Handle;
        fn Rectangle(dc: Handle, l: i32, t: i32, r: i32, b: i32) -> i32;
        fn Ellipse(dc: Handle, l: i32, t: i32, r: i32, b: i32) -> i32;
        fn RoundRect(dc: Handle, l: i32, t: i32, r: i32, b: i32, w: i32, h: i32) -> i32;
        fn CreateFontW(
            height: i32,
            width: i32,
            escapement: i32,
            orientation: i32,
            weight: i32,
            italic: u32,
            underline: u32,
            strikeout: u32,
            charset: u32,
            output: u32,
            clip: u32,
            quality: u32,
            pitch: u32,
            family: *const u16,
        ) -> Handle;
        fn SetTextColor(dc: Handle, color: u32) -> u32;
        fn SetBkMode(dc: Handle, mode: i32) -> i32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleW(name: *const u16) -> Handle;
    }

    enum Command {
        Show(OverlaySpec),
        Clear,
        Stop,
    }
    pub struct Overlay {
        sender: mpsc::Sender<Command>,
        thread: Option<JoinHandle<()>>,
    }
    impl Overlay {
        pub fn new(target: Target) -> Result<Self> {
            let (sender, rx) = mpsc::channel();
            let (ready_tx, ready_rx) = mpsc::sync_channel(1);
            let thread = thread::spawn(move || run(target, rx, ready_tx));
            match ready_rx.recv() {
                Ok(Ok(())) => Ok(Self {
                    sender,
                    thread: Some(thread),
                }),
                Ok(Err(e)) => {
                    let _ = thread.join();
                    Err(e)
                }
                Err(_) => {
                    let _ = thread.join();
                    Err("overlay thread failed to start".into())
                }
            }
        }
        pub fn show(&mut self, spec: OverlaySpec) -> Result<()> {
            spec.validate()?;
            self.sender
                .send(Command::Show(spec))
                .map_err(|_| "overlay thread stopped".into())
        }
        pub fn clear(&mut self) -> Result<()> {
            self.sender
                .send(Command::Clear)
                .map_err(|_| "overlay thread stopped".into())
        }
    }
    impl Drop for Overlay {
        fn drop(&mut self) {
            let _ = self.sender.send(Command::Stop);
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }
    struct State {
        spec: Option<OverlaySpec>,
        expires: Option<Instant>,
    }
    unsafe extern "system" fn find(w: Handle, data: isize) -> i32 {
        unsafe {
            let (target, out) = &mut *(data as *mut (&Target, Handle));
            if IsWindowVisible(w) == 0 {
                return 1;
            }
            let mut class = [0u16; 256];
            let n = GetClassNameW(w, class.as_mut_ptr(), 256).max(0) as usize;
            let mut title = vec![0u16; (GetWindowTextLengthW(w).max(0) as usize) + 1];
            let t = GetWindowTextW(w, title.as_mut_ptr(), title.len() as i32).max(0) as usize;
            if target
                .window_class
                .as_ref()
                .is_some_and(|s| s != &String::from_utf16_lossy(&class[..n]))
                || target
                    .window_title
                    .as_ref()
                    .is_some_and(|s| s != &String::from_utf16_lossy(&title[..t]))
            {
                return 1;
            }
            *out = w;
            0
        }
    }
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }
    unsafe extern "system" fn proc(w: Handle, m: u32, a: usize, b: isize) -> isize {
        unsafe {
            if m == 0x000f {
                let mut ps: Paint = zeroed();
                let dc = BeginPaint(w, &mut ps);
                let state = GetWindowLongPtrW(w, -21) as *const State;
                let mut rect = Rect::default();
                GetClientRect(w, &mut rect);
                let black = CreateSolidBrush(0);
                FillRect(dc, &rect, black);
                DeleteObject(black);
                if let Some(spec) = state.as_ref().and_then(|s| s.spec.as_ref()) {
                    paint(dc, rect, spec);
                }
                EndPaint(w, &ps);
                return 0;
            }
            if m == 0x0014 {
                return 1;
            }
            if m == 0x0084 {
                return -1;
            }
            DefWindowProcW(w, m, a, b)
        }
    }
    unsafe fn paint(dc: Handle, r: Rect, spec: &OverlaySpec) {
        unsafe {
            let mut fonts = std::collections::HashMap::<(u32, String), Handle>::new();
            let w = r.right.max(1) as f64;
            let h = r.bottom.max(1) as f64;
            SetBkMode(dc, 1);
            if let Some(s) = &spec.status {
                draw_text(dc, s, 8, 8, 0xffffff, None);
            }
            for e in &spec.elements {
                let color = match e {
                    OverlayElement::Box { color, .. }
                    | OverlayElement::Circle { color, .. }
                    | OverlayElement::Label { color, .. }
                    | OverlayElement::Point { color, .. } => *color,
                };
                let c = ((color & 255) << 16) | (color & 0xff00) | ((color >> 16) & 255);
                match e {
                    OverlayElement::Label {
                        point,
                        text,
                        font_size,
                        font_family,
                        max_width,
                        shadow,
                        ..
                    } => {
                        let font = if font_size.is_some() || font_family.is_some() {
                            let key = (
                                font_size.unwrap_or(16),
                                font_family.clone().unwrap_or_default(),
                            );
                            *fonts.entry(key.clone()).or_insert_with(|| {
                                CreateFontW(
                                    -(key.0 as i32),
                                    0,
                                    0,
                                    0,
                                    400,
                                    0,
                                    0,
                                    0,
                                    1,
                                    0,
                                    0,
                                    0,
                                    0,
                                    wide(&key.1).as_ptr(),
                                )
                            })
                        } else {
                            null_mut()
                        };
                        let old = if font.is_null() {
                            null_mut()
                        } else {
                            SelectObject(dc, font)
                        };
                        let (x, y) = ((point.x * w) as i32, (point.y * h) as i32);
                        if shadow.unwrap_or(false) {
                            draw_text(dc, text, x + 1, y + 1, 0x010101, *max_width);
                        }
                        draw_text(dc, text, x, y, c, *max_width);
                        if !old.is_null() {
                            SelectObject(dc, old);
                        }
                    }
                    OverlayElement::Box { region, .. } | OverlayElement::Circle { region, .. } => {
                        let (fill, width, radius) = match e {
                            OverlayElement::Box {
                                fill_color,
                                stroke_width,
                                border_radius,
                                ..
                            } => (
                                *fill_color,
                                stroke_width.unwrap_or(2),
                                border_radius.unwrap_or(0),
                            ),
                            _ => (None, 2, 0),
                        };
                        let pen = CreatePen(if width == 0 { 5 } else { 0 }, width as i32, c);
                        let old = SelectObject(dc, pen);
                        let fill_brush = fill.map(|color| {
                            CreateSolidBrush(
                                ((color & 255) << 16) | (color & 0xff00) | ((color >> 16) & 255),
                            )
                        });
                        let brush =
                            SelectObject(dc, fill_brush.unwrap_or_else(|| GetStockObject(5)));
                        let (x, y, x2, y2) = (
                            (region.x * w) as i32,
                            (region.y * h) as i32,
                            ((region.x + region.width) * w) as i32,
                            ((region.y + region.height) * h) as i32,
                        );
                        if matches!(e, OverlayElement::Box { .. }) {
                            if radius == 0 {
                                Rectangle(dc, x, y, x2, y2);
                            } else {
                                RoundRect(
                                    dc,
                                    x,
                                    y,
                                    x2,
                                    y2,
                                    (radius * 2) as i32,
                                    (radius * 2) as i32,
                                );
                            }
                        } else {
                            Ellipse(dc, x, y, x2, y2);
                        }
                        SelectObject(dc, brush);
                        if let Some(fill_brush) = fill_brush {
                            DeleteObject(fill_brush);
                        }
                        SelectObject(dc, old);
                        DeleteObject(pen);
                    }
                    OverlayElement::Point { point, .. } => {
                        let pen = CreatePen(0, 2, c);
                        let old = SelectObject(dc, pen);
                        let brush = SelectObject(dc, GetStockObject(5));
                        let (x, y) = ((point.x * w) as i32, (point.y * h) as i32);
                        Ellipse(dc, x - 3, y - 3, x + 4, y + 4);
                        SelectObject(dc, brush);
                        SelectObject(dc, old);
                        DeleteObject(pen);
                    }
                }
            }
            for font in fonts.into_values() {
                if !font.is_null() {
                    DeleteObject(font);
                }
            }
        }
    }
    unsafe fn draw_text(
        dc: Handle,
        text: &str,
        x: i32,
        y: i32,
        color: u32,
        max_width: Option<u32>,
    ) {
        unsafe {
            SetTextColor(dc, color);
            let s = wide(text);
            let mut r = Rect {
                left: x,
                top: y,
                right: x + max_width.unwrap_or(4096) as i32,
                bottom: y + 1024,
            };
            DrawTextW(
                dc,
                s.as_ptr(),
                -1,
                &mut r,
                0x00000800
                    | if max_width.is_some() {
                        0x20 | 0x8000
                    } else {
                        0
                    },
            );
        }
    }
    fn run(target: Target, rx: mpsc::Receiver<Command>, ready: mpsc::SyncSender<Result<()>>) {
        unsafe {
            let mut target_hwnd: Handle = null_mut();
            let mut pair = (&target, target_hwnd);
            EnumWindows(Some(find), &mut pair as *mut _ as isize);
            target_hwnd = pair.1;
            if target_hwnd.is_null() {
                let _ = ready.send(Err("target window was not found".into()));
                return;
            }
            let instance = GetModuleHandleW(null());
            let name = wide("RustyHotkeyOverlayWindow");
            let class = WndClass {
                style: 0,
                proc: Some(proc),
                extra_class: 0,
                extra_window: 0,
                instance,
                icon: null_mut(),
                cursor: null_mut(),
                background: null_mut(),
                menu: null(),
                class: name.as_ptr(),
            };
            RegisterClassW(&class);
            let w = CreateWindowExW(
                0x00080000 | 0x00000020 | 0x00000080 | 0x08000000 | 0x00000008,
                name.as_ptr(),
                wide("").as_ptr(),
                0x80000000,
                0,
                0,
                1,
                1,
                null_mut(),
                null_mut(),
                instance,
                null_mut(),
            );
            if w.is_null() {
                let _ = ready.send(Err("overlay window creation failed".into()));
                return;
            }
            if SetLayeredWindowAttributes(w, 0, 0, 1) == 0 || SetWindowDisplayAffinity(w, 0x11) == 0
            {
                DestroyWindow(w);
                let _ = ready.send(Err(
                    "overlay transparency or capture exclusion unavailable".into()
                ));
                return;
            }
            let mut state = Box::new(State {
                spec: None,
                expires: None,
            });
            SetWindowLongPtrW(w, -21, (&mut *state as *mut State) as isize);
            let _ = ready.send(Ok(()));
            let mut running = true;
            while running && IsWindow(target_hwnd) != 0 {
                while let Ok(cmd) = rx.try_recv() {
                    match cmd {
                        Command::Show(s) => {
                            state.expires = s.ttl_ms.and_then(|ms| {
                                Instant::now().checked_add(Duration::from_millis(ms))
                            });
                            state.spec = Some(s);
                        }
                        Command::Clear => {
                            state.spec = None;
                            state.expires = None;
                        }
                        Command::Stop => running = false,
                    }
                }
                if state.expires.is_some_and(|t| Instant::now() >= t) {
                    state.spec = None;
                    state.expires = None;
                }
                let mut r = Rect::default();
                let mut p = Pt::default();
                if running
                    && state.spec.is_some()
                    && GetForegroundWindow() == target_hwnd
                    && GetClientRect(target_hwnd, &mut r) != 0
                    && ClientToScreen(target_hwnd, &mut p) != 0
                    && r.right > 0
                    && r.bottom > 0
                {
                    SetWindowPos(
                        w,
                        (-1isize) as Handle,
                        p.x,
                        p.y,
                        r.right,
                        r.bottom,
                        0x0010 | 0x0040,
                    );
                    InvalidateRect(w, null(), 0);
                } else {
                    ShowWindow(w, 0);
                }
                let mut msg: Msg = zeroed();
                while PeekMessageW(&mut msg, null_mut(), 0, 0, 1) != 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                thread::sleep(Duration::from_millis(33));
            }
            SetWindowLongPtrW(w, -21, 0);
            DestroyWindow(w);
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn styled_label_is_bounded_and_legacy_specs_remain_valid() {
            let legacy: OverlaySpec = serde_json::from_str(r#"{"elements":[{"type":"box","region":{"x":0.0,"y":0.0,"width":0.5,"height":0.5},"color":16777215},{"type":"label","point":{"x":0.0,"y":0.0},"text":"hello","color":16777215}]}"#).unwrap();
            legacy.validate().unwrap();
            let styled: OverlaySpec = serde_json::from_str(r#"{"elements":[{"type":"label","point":{"x":0.0,"y":0.0},"text":"A long label that must fit","color":16777215,"font_size":16,"font_family":"Arial","max_width":32,"shadow":true}]}"#).unwrap();
            let image = annotate_capture(
                crate::model::Capture {
                    width: 96,
                    height: 32,
                    bgra: vec![20; 96 * 32 * 4],
                },
                &styled,
            )
            .unwrap();
            assert!(image.bgra.chunks_exact(4).any(|p| p[..3] != [20; 3]));
            for y in 0..32 {
                for x in 33..96 {
                    let i = (y * 96 + x) * 4;
                    assert_eq!(&image.bgra[i..i + 3], &[20; 3]);
                }
            }
            let invalid: OverlaySpec = serde_json::from_str(r#"{"elements":[{"type":"label","point":{"x":0.0,"y":0.0},"text":"x","color":1,"font_size":49}]}"#).unwrap();
            assert!(invalid.validate().is_err());
        }
        #[test]
        fn annotation_renders_onto_capture_and_checks_shape() {
            let capture = crate::model::Capture {
                width: 64,
                height: 64,
                bgra: vec![20; 64 * 64 * 4],
            };
            let spec = OverlaySpec {
                elements: vec![
                    OverlayElement::Box {
                        region: Region {
                            x: 0.1,
                            y: 0.1,
                            width: 0.4,
                            height: 0.4,
                        },
                        color: 0xff0000,
                        fill_color: Some(0x151719),
                        stroke_width: Some(2),
                        border_radius: Some(4),
                    },
                    OverlayElement::Point {
                        point: Point { x: 0.75, y: 0.75 },
                        color: 0x00ff00,
                    },
                ],
                ..Default::default()
            };
            let image = annotate_capture(capture, &spec).unwrap();
            assert_eq!(&image.bgra[..4], &[20; 4]);
            let inside = (12 * 64 + 12) * 4;
            assert_eq!(&image.bgra[inside..inside + 3], &[0x19, 0x17, 0x15]);
            assert!(image.bgra.chunks_exact(4).any(|p| p[..3] == [0, 0, 255]));
            assert!(image.bgra.chunks_exact(4).any(|p| p[..3] == [0, 255, 0]));
            assert!(
                annotate_capture(
                    crate::model::Capture {
                        width: 1,
                        height: 1,
                        bgra: vec![]
                    },
                    &spec
                )
                .is_err()
            );
        }
    }
}
#[cfg(windows)]
pub use native::{Overlay, annotate_capture};
