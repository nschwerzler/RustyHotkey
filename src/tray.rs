//! Explicitly started Windows notification-area controls.
use crate::Result;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct ScriptTrayOptions {
    pub script_path: PathBuf,
    pub engine_pid: u32,
    pub state_dir: PathBuf,
}

// The menu represents the operator's setting, even when target absence or
// observe-only mode temporarily prevents hotkeys from being effective.
fn requested_hotkeys(status: &serde_json::Value) -> bool {
    status
        .get("requested_hotkeys")
        .and_then(|value| value.as_bool())
        .unwrap_or(true)
}

fn restore_focus_for_choice(script: bool, paused: bool, choice: u32) -> bool {
    script && paused && choice == 2
}

pub fn data_dir() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("RHK_DATA_DIR") {
        return Ok(path.into());
    }
    std::env::var_os("APPDATA")
        .map(|p| PathBuf::from(p).join("RustyHotkey"))
        .ok_or_else(|| "APPDATA is unavailable; set RHK_DATA_DIR".into())
}

#[cfg(not(windows))]
pub fn run() -> Result<()> {
    Err("the notification area is available only on Windows".into())
}
#[cfg(not(windows))]
pub fn run_script(_: ScriptTrayOptions) -> Result<()> {
    Err("the notification area is available only on Windows".into())
}

#[cfg(windows)]
mod native {
    use super::*;
    use std::{
        ffi::c_void,
        mem::{size_of, zeroed},
        process::Command,
        ptr::{null, null_mut},
        time::{SystemTime, UNIX_EPOCH},
    };
    type Handle = *mut c_void;
    #[repr(C)]
    #[derive(Default)]
    struct Point {
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
        point: Point,
        private: u32,
    }
    #[repr(C)]
    struct Guid {
        a: u32,
        b: u16,
        c: u16,
        d: [u8; 8],
    }
    #[repr(C)]
    struct NotifyIcon {
        size: u32,
        hwnd: Handle,
        id: u32,
        flags: u32,
        callback: u32,
        icon: Handle,
        tip: [u16; 128],
        state: u32,
        state_mask: u32,
        info: [u16; 256],
        version: u32,
        info_title: [u16; 64],
        info_flags: u32,
        guid: Guid,
        balloon_icon: Handle,
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
        fn PostQuitMessage(code: i32);
        fn GetMessageW(m: *mut Msg, w: Handle, min: u32, max: u32) -> i32;
        fn TranslateMessage(m: *const Msg) -> i32;
        fn DispatchMessageW(m: *const Msg) -> isize;
        fn CreatePopupMenu() -> Handle;
        fn AppendMenuW(menu: Handle, flags: u32, id: usize, text: *const u16) -> i32;
        fn DestroyMenu(menu: Handle) -> i32;
        fn TrackPopupMenu(
            menu: Handle,
            flags: u32,
            x: i32,
            y: i32,
            reserved: i32,
            w: Handle,
            rect: *const c_void,
        ) -> u32;
        fn SetForegroundWindow(w: Handle) -> i32;
        fn GetForegroundWindow() -> Handle;
        fn IsWindow(w: Handle) -> i32;
        fn GetCursorPos(p: *mut Point) -> i32;
        fn CreateIcon(
            instance: Handle,
            w: i32,
            h: i32,
            planes: u8,
            bits: u8,
            and_bits: *const u8,
            xor_bits: *const u8,
        ) -> Handle;
        fn DestroyIcon(icon: Handle) -> i32;
        fn SetWindowLongPtrW(w: Handle, index: i32, value: isize) -> isize;
        fn GetWindowLongPtrW(w: Handle, index: i32) -> isize;
        fn SetTimer(w: Handle, id: usize, ms: u32, callback: Handle) -> usize;
    }
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn Shell_NotifyIconW(message: u32, data: *mut NotifyIcon) -> i32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleW(name: *const u16) -> Handle;
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
        fn WaitForSingleObject(h: Handle, ms: u32) -> u32;
        fn CloseHandle(h: Handle) -> i32;
    }
    const CALLBACK: u32 = 0x8001;
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }
    fn tooltip(n: &mut NotifyIcon, text: &str) {
        n.tip.fill(0);
        for (dst, src) in n.tip.iter_mut().take(127).zip(text.encode_utf16()) {
            *dst = src;
        }
    }
    unsafe fn icon(color: u32, seed: u64) -> Handle {
        unsafe {
            // A ring and script-specific geometric mark, generated in memory.
            let mut pixels = [0u8; 32 * 32 * 4];
            let mut mask = [0xffu8; 32 * 4];
            for y in 0..32 {
                for x in 0..32 {
                    let dx = x as f32 - 15.5;
                    let dy = y as f32 - 15.5;
                    let d = (dx * dx + dy * dy).sqrt();
                    if d < 14.5 {
                        let i = (y * 32 + x) * 4;
                        let mark = if (11..21).contains(&x) && (11..21).contains(&y) {
                            let bit = ((y - 11) / 2) * 5 + (x - 11) / 2;
                            (seed >> bit) & 1 != 0
                        } else {
                            false
                        };
                        let (r, g, b) = if d > 10.5 || mark {
                            ((color >> 16) as u8, (color >> 8) as u8, color as u8)
                        } else {
                            (30, 34, 43)
                        };
                        pixels[i] = b;
                        pixels[i + 1] = g;
                        pixels[i + 2] = r;
                        pixels[i + 3] = 255;
                        mask[y * 4 + x / 8] &= !(0x80 >> (x % 8));
                    }
                }
            }
            CreateIcon(
                GetModuleHandleW(null()),
                32,
                32,
                1,
                32,
                mask.as_ptr(),
                pixels.as_ptr(),
            )
        }
    }
    fn signal_value(dir: PathBuf, name: &str, enabled: Option<bool>) -> Result<()> {
        std::fs::create_dir_all(&dir).map_err(|e| format!("create data folder: {e}"))?;
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| format!("clock: {e}"))?
            .as_nanos();
        let temp = dir.join(format!("pause.{}.{}.tmp", std::process::id(), nonce));
        let contents = match enabled {
            Some(enabled) => format!("{{\"enabled\":{enabled},\"nonce\":{nonce}}}"),
            None => nonce.to_string(),
        };
        std::fs::write(&temp, contents).map_err(|e| format!("write control signal: {e}"))?;
        if let Err(e) = std::fs::rename(&temp, dir.join(name)) {
            let _ = std::fs::remove_file(temp);
            return Err(format!("publish control signal: {e}"));
        }
        Ok(())
    }
    fn signal(dir: PathBuf, name: &str) -> Result<()> {
        signal_value(dir, name, None)
    }
    fn pause() -> Result<()> {
        signal(data_dir()?, "pause.signal")
    }
    fn open_ui() -> Result<()> {
        Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
            .arg("ui")
            .spawn()
            .map_err(|e| format!("launch UI: {e}"))?;
        Ok(())
    }
    fn open_data() -> Result<()> {
        let dir = data_dir()?;
        std::fs::create_dir_all(&dir).map_err(|e| format!("create data folder: {e}"))?;
        Command::new("explorer.exe")
            .arg(dir)
            .spawn()
            .map_err(|e| format!("open data folder: {e}"))?;
        Ok(())
    }
    unsafe fn menu(w: Handle, script: bool, hotkeys: bool, paused: bool) -> u32 {
        unsafe {
            let menu = CreatePopupMenu();
            if menu.is_null() {
                return 0;
            }
            if script {
                AppendMenuW(
                    menu,
                    if paused { 8 } else { 0 },
                    2,
                    wide("Pause Script").as_ptr(),
                );
                AppendMenuW(
                    menu,
                    if hotkeys { 8 } else { 0 },
                    5,
                    wide("Hotkeys Enabled").as_ptr(),
                );
                AppendMenuW(menu, 0, 3, wide("Edit Script").as_ptr());
                AppendMenuW(menu, 0, 4, wide("Exit Script").as_ptr());
            } else {
                AppendMenuW(menu, 0, 1, wide("Open RHK BOT").as_ptr());
                AppendMenuW(menu, 0, 2, wide("Pause all automation").as_ptr());
                AppendMenuW(menu, 0, 3, wide("Open data folder").as_ptr());
                AppendMenuW(menu, 0x800, 0, null());
                AppendMenuW(menu, 0, 4, wide("Exit").as_ptr());
            }
            let mut p = Point::default();
            GetCursorPos(&mut p);
            let previous_foreground = GetForegroundWindow();
            SetForegroundWindow(w);
            let choice = TrackPopupMenu(menu, 0x100 | 0x2, p.x, p.y, 0, w, null());
            DestroyMenu(menu);
            if restore_focus_for_choice(script, paused, choice)
                && !previous_foreground.is_null()
                && previous_foreground != w
                && IsWindow(previous_foreground) != 0
            {
                SetForegroundWindow(previous_foreground);
            }
            choice
        }
    }
    struct State {
        options: Option<ScriptTrayOptions>,
        process: Handle,
        image: Handle,
        seed: u64,
        color: u32,
        hotkeys: bool,
        paused: bool,
    }
    unsafe extern "system" fn proc(w: Handle, m: u32, _a: usize, b: isize) -> isize {
        unsafe {
            if m == 0x0113 {
                let state = (GetWindowLongPtrW(w, -21) as *mut State).as_mut();
                if let Some(s) = state {
                    if !s.process.is_null() && WaitForSingleObject(s.process, 0) == 0 {
                        DestroyWindow(w);
                        return 0;
                    }
                    if let Some(o) = &s.options {
                        let p = o
                            .state_dir
                            .join("control")
                            .join(format!("{}.status.json", o.engine_pid));
                        let status = std::fs::read(p)
                            .ok()
                            .and_then(|v| serde_json::from_slice::<serde_json::Value>(&v).ok());
                        let get = |key: &str, default: bool| {
                            status
                                .as_ref()
                                .and_then(|v| v.get(key))
                                .and_then(|v| v.as_bool())
                                .unwrap_or(default)
                        };
                        s.paused = get("paused", false);
                        s.hotkeys = status.as_ref().map(requested_hotkeys).unwrap_or(true);
                        let dormant = get("dormant", false);
                        let error = get("error", false);
                        let (color, label) = if error {
                            (0xe55d62, "error")
                        } else if dormant {
                            (0x9098a3, "dormant")
                        } else if s.paused {
                            (0xf4aa39, "paused")
                        } else if !get("hotkeys_enabled", true) {
                            (0x64a9f3, "hotkeys disabled")
                        } else if get("armed", true) {
                            (0x4cc78a, "running")
                        } else {
                            (0x9098a3, "stopped")
                        };
                        let mut n: NotifyIcon = zeroed();
                        n.size = size_of::<NotifyIcon>() as u32;
                        n.hwnd = w;
                        n.id = 1;
                        n.flags = 4 | 0x80;
                        let name = o
                            .script_path
                            .file_name()
                            .map(|v| v.to_string_lossy())
                            .unwrap_or_default();
                        tooltip(&mut n, &format!("{name} — {label}"));
                        if color != s.color {
                            let next = icon(color, s.seed);
                            if !next.is_null() {
                                n.flags |= 2;
                                n.icon = next;
                                if Shell_NotifyIconW(1, &mut n) != 0 {
                                    DestroyIcon(s.image);
                                    s.image = next;
                                    s.color = color;
                                } else {
                                    DestroyIcon(next);
                                }
                            }
                        } else {
                            Shell_NotifyIconW(1, &mut n);
                        }
                    }
                }
                return 0;
            }
            if m == CALLBACK {
                let state = (GetWindowLongPtrW(w, -21) as *mut State).as_mut();
                let script = state.as_ref().is_some_and(|s| s.options.is_some());
                let event = (b as u32) & 0xffff;
                let choice = match event {
                    0x0203 | 0x0400 if !script => 1,
                    0x0205 | 0x007b => menu(
                        w,
                        script,
                        state.as_ref().is_some_and(|s| s.hotkeys),
                        state.as_ref().is_some_and(|s| s.paused),
                    ),
                    _ => 0,
                };
                let result = if let Some(o) = state.as_ref().and_then(|s| s.options.as_ref()) {
                    let dir = o.state_dir.join("control");
                    match choice {
                        2 => signal(
                            dir,
                            &format!(
                                "{}.{}",
                                o.engine_pid,
                                if state.as_ref().unwrap().paused {
                                    "resume"
                                } else {
                                    "pause"
                                }
                            ),
                        ),
                        5 => signal_value(
                            dir,
                            &format!("{}.hotkeys", o.engine_pid),
                            Some(!state.as_ref().unwrap().hotkeys),
                        ),
                        3 => Command::new("notepad.exe")
                            .arg(&o.script_path)
                            .spawn()
                            .map(|_| ())
                            .map_err(|e| format!("open script editor: {e}")),
                        4 => signal(dir.clone(), &format!("{}.pause", o.engine_pid))
                            .and_then(|_| signal(dir, &format!("{}.exit", o.engine_pid))),
                        _ => Ok(()),
                    }
                } else {
                    match choice {
                        1 => open_ui(),
                        2 => pause(),
                        3 => open_data(),
                        4 => {
                            let mut n: NotifyIcon = zeroed();
                            n.size = size_of::<NotifyIcon>() as u32;
                            n.hwnd = w;
                            n.id = 1;
                            Shell_NotifyIconW(2, &mut n);
                            DestroyWindow(w);
                            Ok(())
                        }
                        _ => Ok(()),
                    }
                };
                if choice == 2 {
                    let mut n: NotifyIcon = zeroed();
                    n.size = size_of::<NotifyIcon>() as u32;
                    n.hwnd = w;
                    n.id = 1;
                    n.flags = 4 | 0x80;
                    tooltip(
                        &mut n,
                        if result.is_ok() {
                            "RHK — state change requested"
                        } else {
                            "RHK — state change request failed"
                        },
                    );
                    Shell_NotifyIconW(1, &mut n);
                }
                if let Err(e) = result {
                    eprintln!("tray: {e}");
                }
                return 0;
            }
            if m == 2 {
                PostQuitMessage(0);
                return 0;
            }
            DefWindowProcW(w, m, _a, b)
        }
    }
    pub fn run() -> Result<()> {
        run_inner(None)
    }
    pub fn run_script(options: ScriptTrayOptions) -> Result<()> {
        run_inner(Some(options))
    }
    fn run_inner(options: Option<ScriptTrayOptions>) -> Result<()> {
        unsafe {
            let instance = GetModuleHandleW(null());
            let name = wide("RustyHotkeyTrayWindow");
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
                0,
                name.as_ptr(),
                wide("RHK tray").as_ptr(),
                0,
                0,
                0,
                0,
                0,
                null_mut(),
                null_mut(),
                instance,
                null_mut(),
            );
            if w.is_null() {
                return Err("create tray window failed".into());
            }
            let process = if let Some(o) = &options {
                let p = OpenProcess(0x00100000, 0, o.engine_pid);
                if p.is_null() {
                    DestroyWindow(w);
                    return Err("could not observe script process".into());
                }
                p
            } else {
                null_mut()
            };
            let seed = options
                .as_ref()
                .map(|o| {
                    o.script_path
                        .to_string_lossy()
                        .bytes()
                        .fold(0xcbf29ce484222325u64, |h, b| {
                            (h ^ b as u64).wrapping_mul(0x100000001b3)
                        })
                })
                .unwrap_or(0);
            let color = if options.is_some() {
                0x4cc78a
            } else {
                0xf4aa39
            };
            let image = icon(color, seed);
            if image.is_null() {
                if !process.is_null() {
                    CloseHandle(process);
                }
                DestroyWindow(w);
                return Err("create tray icon failed".into());
            }
            let mut state = Box::new(State {
                options,
                process,
                image,
                seed,
                color,
                hotkeys: true,
                paused: false,
            });
            SetWindowLongPtrW(w, -21, (&mut *state as *mut State) as isize);
            let mut n: NotifyIcon = zeroed();
            n.size = size_of::<NotifyIcon>() as u32;
            n.hwnd = w;
            n.id = 1;
            n.flags = 1 | 2 | 4 | 0x80;
            n.callback = CALLBACK;
            n.icon = image;
            let title = state
                .options
                .as_ref()
                .and_then(|o| o.script_path.file_name())
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "RHK — ready".into());
            tooltip(&mut n, &title);
            if Shell_NotifyIconW(0, &mut n) == 0 {
                SetWindowLongPtrW(w, -21, 0);
                DestroyIcon(image);
                if !process.is_null() {
                    CloseHandle(process);
                }
                DestroyWindow(w);
                return Err("add notification icon failed".into());
            }
            n.version = 4;
            if Shell_NotifyIconW(4, &mut n) == 0 {
                Shell_NotifyIconW(2, &mut n);
                SetWindowLongPtrW(w, -21, 0);
                DestroyIcon(image);
                if !process.is_null() {
                    CloseHandle(process);
                }
                DestroyWindow(w);
                return Err("set tray icon version failed".into());
            }
            if !process.is_null() {
                SetTimer(w, 1, 1000, null_mut());
            }
            let mut msg: Msg = zeroed();
            let mut error = None;
            loop {
                let status = GetMessageW(&mut msg, null_mut(), 0, 0);
                if status == 0 {
                    break;
                }
                if status < 0 {
                    error = Some("tray message loop failed".to_string());
                    break;
                }
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            Shell_NotifyIconW(2, &mut n);
            SetWindowLongPtrW(w, -21, 0);
            DestroyIcon(state.image);
            if !process.is_null() {
                CloseHandle(process);
            }
            DestroyWindow(w);
            if let Some(e) = error { Err(e) } else { Ok(()) }
        }
    }
}
#[cfg(windows)]
pub use native::{run, run_script};

#[cfg(test)]
mod tests {
    use super::{requested_hotkeys, restore_focus_for_choice};
    use serde_json::json;

    #[test]
    fn menu_uses_requested_hotkeys_when_temporarily_inactive() {
        assert!(requested_hotkeys(&json!({
            "requested_hotkeys": true, "hotkeys_enabled": false, "dormant": true
        })));
        assert!(!requested_hotkeys(&json!({
            "requested_hotkeys": false, "hotkeys_enabled": false
        })));
    }

    #[test]
    fn only_explicit_script_resume_restores_focus() {
        assert!(restore_focus_for_choice(true, true, 2));
        assert!(!restore_focus_for_choice(true, false, 2));
        assert!(!restore_focus_for_choice(true, true, 5));
        assert!(!restore_focus_for_choice(false, true, 2));
        assert!(!restore_focus_for_choice(true, true, 0));
    }
}
