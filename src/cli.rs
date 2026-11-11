use crate::{
    Backend, Result,
    model::*,
    platform::WindowsBackend,
    recording,
    runtime::{Engine, validate_profile},
    workflow,
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

const HELP: &str = "RHK — RustyHotkey (Windows x64)\n\nCommands:\n  rhk help | capabilities | tools\n  rhk validate profile|workflow|recording FILE\n  rhk preview WORKFLOW\n  rhk run|rpc PROFILE [--script PATH] [--no-tray]\n  rhk overlay PROFILE WIDGET.lua [WIDGET.lua...]\n  rhk ui [--no-open] [--port PORT] [--data-dir PATH]\n  rhk tray [--script PATH --engine-pid PID]\n  rhk fetch URL [--database PATH] [--ttl-ms N] [--allow-http]\n  rhk record PROFILE OUTPUT [--include-text]\n  rhk replay PROFILE RECORDING [SPEED]\n  rhk db DATABASE\n\nProfiles and runtime data are caller supplied. run starts paused; the configured\nphysical resume key arms the engine. rpc uses newline-delimited JSON requests.\nSee docs/usage.md, docs/python.md and llms.txt for the API contract.";

pub fn run(args: impl Iterator<Item = String>) -> Result<()> {
    let args: Vec<_> = args.collect();
    match args.first().map(String::as_str).unwrap_or("help") {
        "help" | "--help" | "-h" => {
            println!("{HELP}");
            Ok(())
        }
        "capabilities" => output(
            &json!({"name":"rhk","version":env!("CARGO_PKG_VERSION"),"architecture":"x86_64","platform":"windows","interfaces":["rust","python","cli","json_lines"],"capabilities":["conditions","hotkeys","input","pixels","clipboard","capture","ocr","notifications","workflows","recording","replay","database","cache"]}),
        ),
        "validate" => {
            let kind = arg(&args, 1, "kind")?;
            let path = arg(&args, 2, "file")?;
            match kind {
                "profile" => validate_profile(&read_json::<Profile>(path)?)?,
                "workflow" => workflow::validate(&read_json::<Workflow>(path)?)?,
                "recording" => recording::validate(&read_json::<Recording>(path)?)?,
                _ => return Err("kind must be profile, workflow or recording".into()),
            };
            output(&json!({"valid":true,"kind":kind}))
        }
        "preview" => output(&workflow::dry_run(&read_json(arg(&args, 1, "workflow")?)?)?),
        "rpc" | "run" => {
            let path = arg(&args, 1, "profile")?;
            let profile: Profile = read_json(path)?;
            let dir = state_dir()?;
            let _status = StatusFile::new(&dir)?;
            let backend = WindowsBackend::new(profile.target.clone())?;
            let mut engine = Engine::new(backend, profile, dir.clone())?;
            let script = option(&args, "--script")?.unwrap_or(path);
            if !args.iter().any(|a| a == "--no-tray") {
                spawn_tray(script)?;
            }
            if args[0] == "rpc" {
                engine.set_watchdog(Some(Duration::from_secs(2)));
                rpc(engine, _status)
            } else {
                eprintln!("RHK is paused. Use the configured physical controls.");
                loop {
                    if let Err(error) = engine.tick() {
                        eprintln!("RHK paused: {error}");
                    }
                    _status.write(&engine.status())?;
                    if engine.exit_requested() {
                        engine.pause()?;
                        break;
                    }
                    thread::sleep(Duration::from_millis(engine.profile.tick_ms.min(10)));
                }
                Ok(())
            }
        }
        "record" => record_command(&args),
        "replay" => {
            let profile: Profile = read_json(arg(&args, 1, "profile")?)?;
            let recording: Recording = read_json(arg(&args, 2, "recording")?)?;
            let speed = args
                .get(3)
                .map(|s| s.parse::<f64>())
                .transpose()
                .map_err(|e| e.to_string())?
                .unwrap_or(1.0);
            let actions = recording::plan(&recording, speed)?;
            let backend = WindowsBackend::new(profile.target.clone())?;
            let mut engine = Engine::new(backend, profile, state_dir()?)?;
            engine.arm()?;
            engine.execute(&actions)?;
            engine.pause()?;
            output(&json!({"complete":true}))
        }
        "db" => database_rpc(Path::new(arg(&args, 1, "database")?)),
        "fetch" => fetch_command(&args),
        "tools" => output(&tool_catalog()),
        "ui" => ui_command(&args),
        "tray" => {
            if let Some(script) = option(&args, "--script")? {
                let pid = option(&args, "--engine-pid")?
                    .ok_or("missing --engine-pid")?
                    .parse()
                    .map_err(|_| "invalid engine PID")?;
                crate::tray::run_script(crate::tray::ScriptTrayOptions {
                    script_path: PathBuf::from(script),
                    engine_pid: pid,
                    state_dir: state_dir()?,
                })
            } else {
                crate::tray::run()
            }
        }
        "overlay" => overlay_command(&args),
        other => Err(format!("unknown command {other:?}; run rhk help")),
    }
}
fn arg<'a>(args: &'a [String], index: usize, name: &str) -> Result<&'a str> {
    args.get(index)
        .map(String::as_str)
        .ok_or_else(|| format!("missing {name}; run rhk help"))
}
fn option<'a>(args: &'a [String], name: &str) -> Result<Option<&'a str>> {
    match args.iter().position(|s| s == name) {
        Some(i) => Ok(Some(arg(args, i + 1, name)?)),
        None => Ok(None),
    }
}
fn read_json<T: DeserializeOwned>(path: impl AsRef<Path>) -> Result<T> {
    let data = fs::read(path.as_ref()).map_err(|e| format!("{}: {e}", path.as_ref().display()))?;
    if data.len() > 8 * 1024 * 1024 {
        return Err("JSON file exceeds 8 MiB".into());
    }
    serde_json::from_slice(&data).map_err(|e| e.to_string())
}
fn field<T: DeserializeOwned>(request: &Value, key: &str) -> Result<T> {
    serde_json::from_value(
        request
            .get(key)
            .cloned()
            .ok_or_else(|| format!("missing {key}"))?,
    )
    .map_err(|e| format!("{key}: {e}"))
}
fn output(value: &Value) -> Result<()> {
    let mut out = io::stdout().lock();
    serde_json::to_writer(&mut out, value).map_err(|e| e.to_string())?;
    writeln!(out).map_err(|e| e.to_string())?;
    out.flush().map_err(|e| e.to_string())
}
fn state_dir() -> Result<PathBuf> {
    let root = std::env::var_os("RHK_DATA_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("APPDATA").map(|p| PathBuf::from(p).join("RustyHotkey")))
        .ok_or("set RHK_DATA_DIR or APPDATA")?;
    Ok(root)
}

fn rpc<B: Backend>(engine: Engine<B>, status: StatusFile) -> Result<()> {
    let control = engine.control_handle();
    let (tx, rx) = mpsc::sync_channel::<Value>(64);
    thread::spawn(move || {
        let stdin = io::stdin();
        for line in stdin.lock().lines() {
            let line = match line {
                Ok(x) => x,
                Err(_) => break,
            };
            if line.len() > 1024 * 1024 {
                let _ = output(&json!({"id":null,"ok":false,"error":"request exceeds 1 MiB"}));
                continue;
            }
            match serde_json::from_str::<Value>(&line) {
                Ok(value) => {
                    match value.get("op").and_then(Value::as_str) {
                        Some("pause") => control.request_pause(),
                        Some("abort") => control.request_abort(),
                        Some("heartbeat") => {
                            control.heartbeat();
                            let id = value.get("id").cloned().unwrap_or(Value::Null);
                            if output(&json!({"id":id,"ok":true,"result":{"alive":true}})).is_err()
                            {
                                break;
                            }
                            continue;
                        }
                        _ => {}
                    }
                    if tx.send(value).is_err() {
                        break;
                    }
                }
                Err(error) => {
                    let _ = output(&json!({"id":null,"ok":false,"error":error.to_string()}));
                }
            }
        }
        control.request_pause();
    });
    let mut session = RpcSession::new(engine);
    loop {
        match rx.recv_timeout(Duration::from_millis(10)) {
            Ok(request) => {
                let id = request.get("id").cloned().unwrap_or(Value::Null);
                let result = session.dispatch(&request);
                output(&match result {
                    Ok(value) => json!({"id":id,"ok":true,"result":value}),
                    Err(error) => json!({"id":id,"ok":false,"error":error}),
                })?;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if let Err(error) = session.engine.tick() {
                    let _ = session.engine.pause();
                    eprintln!("RHK paused: {error}");
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                session.engine.pause()?;
                break;
            }
        }
        let rendered = session.render();
        if rendered["diagnostics"]
            .as_array()
            .is_some_and(|v| !v.is_empty())
            || rendered.get("overlay_error").is_some()
        {
            eprintln!("widgets: {rendered}");
        }
        status.write(&session.engine.status())?;
        if session.engine.exit_requested() {
            session.engine.pause()?;
            break;
        }
    }
    Ok(())
}
fn dispatch<B: Backend>(engine: &mut Engine<B>, request: &Value) -> Result<Value> {
    let op = request
        .get("op")
        .and_then(Value::as_str)
        .ok_or("missing op")?;
    match op {
        "status" => {
            let mut value = engine.status();
            value["present"] = json!(engine.backend.target_present()?);
            value["focused"] = json!(engine.backend.focused()?);
            Ok(value)
        }
        "arm" => {
            engine.arm()?;
            Ok(engine.status())
        }
        "pause" => {
            engine.pause()?;
            Ok(engine.status())
        }
        "abort" => {
            engine.abort()?;
            Ok(engine.status())
        }
        "heartbeat" => {
            engine.heartbeat();
            Ok(json!({"alive":true}))
        }
        "hotkeys" => {
            engine.set_hotkeys_enabled(field(request, "enabled")?);
            Ok(engine.status())
        }
        "input_events" => {
            let after = match request.get("after") {
                None => 0,
                Some(value) => value
                    .as_u64()
                    .ok_or("after must be a nonnegative integer")?,
            };
            Ok(engine.input_events(after))
        }
        "tick" => {
            engine.tick()?;
            Ok(engine.status())
        }
        "execute" => {
            engine.execute(&field::<Vec<Action>>(request, "actions")?)?;
            Ok(engine.status())
        }
        "evaluate" => Ok(json!({"matches":engine.matches(&field(request,"condition")?)?})),
        "pixel" => Ok(json!({"color":engine.backend.pixel(field(request,"point")?)?})),
        "clipboard" => Ok(json!({"text":engine.backend.clipboard()?})),
        "screenshot" => {
            let capture = engine.backend.screenshot(
                request
                    .get("region")
                    .filter(|v| !v.is_null())
                    .map(|x| serde_json::from_value(x.clone()).map_err(|e| e.to_string()))
                    .transpose()?,
            )?;
            let width = capture.width;
            let height = capture.height;
            let file = if let Some(path) = request.get("path").and_then(Value::as_str) {
                PathBuf::from(path)
            } else {
                state_dir()?.join("captures").join(format!(
                    "capture-{}.bmp",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_err(|e| e.to_string())?
                        .as_nanos()
                ))
            };
            write_bmp(&file, &capture)?;
            Ok(json!({"width":width,"height":height,"path":file}))
        }
        "ocr" => Ok(
            json!({"words":engine.backend.ocr(request.get("region").filter(|v|!v.is_null()).map(|x|serde_json::from_value(x.clone()).map_err(|e|e.to_string())).transpose()?)?}),
        ),
        "notify" => {
            engine.backend.notify(
                &field::<String>(request, "message")?,
                field(request, "ttl_ms")?,
            )?;
            Ok(json!({"shown":true}))
        }
        "workflow" => {
            workflow::run(engine, &field(request, "workflow")?)?;
            Ok(json!({"complete":true}))
        }
        _ => Err(format!("unknown operation {op:?}")),
    }
}
fn write_bmp(path: &Path, capture: &Capture) -> Result<()> {
    let size = capture
        .width
        .checked_mul(capture.height)
        .and_then(|n| n.checked_mul(4))
        .ok_or("capture too large")?;
    if capture.bgra.len() != size as usize {
        return Err("invalid capture buffer".into());
    }
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut bytes = Vec::with_capacity(size as usize + 54);
    bytes.extend(b"BM");
    bytes.extend((54 + size).to_le_bytes());
    bytes.extend([0; 4]);
    bytes.extend(54u32.to_le_bytes());
    bytes.extend(40u32.to_le_bytes());
    bytes.extend((capture.width as i32).to_le_bytes());
    bytes.extend((-(capture.height as i32)).to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(32u16.to_le_bytes());
    bytes.extend([0; 24]);
    bytes.extend(&capture.bgra);
    fs::write(path, bytes).map_err(|e| e.to_string())
}
fn record_command(args: &[String]) -> Result<()> {
    let profile: Profile = read_json(arg(args, 1, "profile")?)?;
    validate_profile(&profile)?;
    let output_file = PathBuf::from(arg(args, 2, "output")?);
    if output_file.exists() {
        return Err("recording output already exists".into());
    }
    let mut backend = WindowsBackend::new(profile.target.clone())?;
    if !backend.focused()? {
        return Err("target must be focused before recording".into());
    }
    let mut recorder = recording::Recorder::new(
        &profile.controls,
        args.iter().any(|s| s == "--include-text"),
    );
    let start = Instant::now();
    eprintln!(
        "Recording; configured pause or abort key stops. Printable keys are omitted unless --include-text is supplied."
    );
    loop {
        if !backend.focused()? {
            return Err("target lost focus during recording".into());
        }
        if backend.key_down(profile.controls.abort)? {
            return Err("recording aborted".into());
        }
        if backend.key_down(profile.controls.pause)? {
            break;
        }
        let mut keys = Vec::new();
        for key in 1..=255 {
            if backend.key_down(key)? {
                keys.push(key)
            }
        }
        recorder.sample(start.elapsed().as_millis() as u64, &keys, backend.cursor()?)?;
        thread::sleep(Duration::from_millis(profile.tick_ms.min(10)));
    }
    let recording = recorder.finish();
    recording::validate(&recording)?;
    if let Some(parent) = output_file.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(
        &output_file,
        serde_json::to_vec_pretty(&recording).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    output(&json!({"path":output_file,"events":recording.events.len()}))
}
fn database_rpc(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let store = crate::store::Store::open(path)?;
    for line in io::stdin().lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        let request: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(error) => {
                output(&json!({"id":null,"ok":false,"error":error.to_string()}))?;
                continue;
            }
        };
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let result = dispatch_database(&store, &request);
        output(&match result {
            Ok(value) => json!({"id":id,"ok":true,"result":value}),
            Err(error) => json!({"id":id,"ok":false,"error":error}),
        })?;
    }
    Ok(())
}
fn dispatch_database(store: &crate::store::Store, request: &Value) -> Result<Value> {
    let params = || -> Result<Vec<Value>> {
        request
            .get("params")
            .map(|v| serde_json::from_value(v.clone()).map_err(|e| e.to_string()))
            .unwrap_or(Ok(vec![]))
    };
    let text = |key: &str| -> Result<String> { field::<String>(request, key) };
    match request
        .get("op")
        .and_then(Value::as_str)
        .ok_or("missing op")?
    {
        "execute" => Ok(json!(store.execute(&text("sql")?, &params()?)?)),
        "query" => Ok(json!(store.query(&text("sql")?, &params()?)?)),
        "transaction" => {
            let statements: Vec<Value> = field(request, "statements")?;
            let statements = statements
                .iter()
                .map(|s| {
                    Ok(crate::store::Statement::new(
                        field::<String>(s, "sql")?,
                        s.get("params")
                            .map(|v| serde_json::from_value(v.clone()).map_err(|e| e.to_string()))
                            .unwrap_or(Ok(vec![]))?,
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(json!(store.transaction(&statements)?))
        }
        "migrate" => Ok(json!(
            store.migrate(field(request, "version")?, &text("sql")?)?
        )),
        "cache_put" => {
            store.cache_put(
                &text("namespace")?,
                &text("key")?,
                field(request, "value")?,
                field(request, "ttl_ms")?,
            )?;
            Ok(Value::Null)
        }
        "cache_get" => Ok(store
            .cache_get(&text("namespace")?, &text("key")?)?
            .unwrap_or(Value::Null)),
        "cache_invalidate" => {
            store.cache_invalidate(&text("namespace")?, &text("key")?)?;
            Ok(Value::Null)
        }
        "cache_clear" => {
            store.cache_clear(&text("namespace")?)?;
            Ok(Value::Null)
        }
        "cache_prune" => Ok(json!(store.cache_prune()?)),
        "fetch" => Ok(serde_json::to_value(crate::web::fetch(
            &field(request, "request")?,
            Some(store),
        )?)
        .map_err(|e| e.to_string())?),
        "status" => Ok(json!({"engine":"turso"})),
        op => Err(format!("unknown database operation {op:?}")),
    }
}

struct StatusFile {
    dir: PathBuf,
    path: PathBuf,
    last: std::cell::RefCell<Option<Value>>,
}
impl StatusFile {
    fn new(root: &Path) -> Result<Self> {
        let dir = root.join("control");
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let pid = std::process::id();
        for suffix in ["pause", "exit", "hotkeys", "status.json"] {
            let _ = fs::remove_file(dir.join(format!("{pid}.{suffix}")));
        }
        Ok(Self {
            path: dir.join(format!("{pid}.status.json")),
            dir,
            last: std::cell::RefCell::new(None),
        })
    }
    fn write(&self, value: &Value) -> Result<()> {
        if self.last.borrow().as_ref() == Some(value) {
            return Ok(());
        }
        let temp = self.dir.join(format!("{}.status.tmp", std::process::id()));
        fs::write(&temp, serde_json::to_vec(value).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        fs::rename(temp, &self.path).map_err(|e| e.to_string())?;
        *self.last.borrow_mut() = Some(value.clone());
        Ok(())
    }
}
impl Drop for StatusFile {
    fn drop(&mut self) {
        let pid = std::process::id();
        for suffix in ["pause", "exit", "hotkeys", "status.json", "status.tmp"] {
            let _ = fs::remove_file(self.dir.join(format!("{pid}.{suffix}")));
        }
    }
}

fn spawn_tray(script: &str) -> Result<()> {
    Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
        .args([
            "tray",
            "--script",
            script,
            "--engine-pid",
            &std::process::id().to_string(),
        ])
        .spawn()
        .map_err(|e| format!("start script tray: {e}"))?;
    Ok(())
}
fn ui_command(args: &[String]) -> Result<()> {
    let python = std::env::var_os("RHK_PYTHON").unwrap_or_else(|| "python".into());
    let mut command = Command::new(python);
    command.args(["-m", "rustyhotkey.bot_server"]);
    command.args(&args[1..]);
    if let Some(path) = std::env::var_os("RHK_PYTHON_PATH") {
        command.env("PYTHONPATH", path);
    } else {
        let path = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.join("python")))
            .filter(|p| p.join("rustyhotkey").is_dir())
            .or_else(|| {
                std::env::current_dir()
                    .ok()
                    .map(|dir| dir.join("python"))
                    .filter(|p| p.join("rustyhotkey").is_dir())
            });
        if let Some(path) = path {
            command.env("PYTHONPATH", path);
        }
    }
    let status = command
        .status()
        .map_err(|e| format!("start Python UI: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("Python UI exited with {status}"))
    }
}
fn fetch_command(args: &[String]) -> Result<()> {
    let ttl = option(args, "--ttl-ms")?
        .map(str::parse::<u64>)
        .transpose()
        .map_err(|e| e.to_string())?;
    let req = crate::web::FetchRequest {
        url: arg(args, 1, "url")?.to_string(),
        timeout_ms: 15_000,
        max_bytes: 1024 * 1024,
        cache_ttl_ms: ttl,
        allow_http: args.iter().any(|a| a == "--allow-http"),
    };
    let store = option(args, "--database")?
        .map(crate::store::Store::open)
        .transpose()?;
    output(
        &serde_json::to_value(crate::web::fetch(&req, store.as_ref())?)
            .map_err(|e| e.to_string())?,
    )
}

struct RpcSession<B: Backend> {
    engine: Engine<B>,
    widgets: crate::lua::WidgetManager,
    overlay: Option<crate::overlay::Overlay>,
    widget_state: Value,
    last_render: Instant,
}
impl<B: Backend> RpcSession<B> {
    fn new(engine: Engine<B>) -> Self {
        Self {
            engine,
            widgets: crate::lua::WidgetManager::new(&[] as &[&Path]),
            overlay: None,
            widget_state: json!({}),
            last_render: Instant::now() - Duration::from_secs(1),
        }
    }
    fn dispatch(&mut self, request: &Value) -> Result<Value> {
        match request
            .get("op")
            .and_then(Value::as_str)
            .ok_or("missing op")?
        {
            "widgets" => {
                let paths: Vec<PathBuf> = field(request, "paths")?;
                let state = request.get("state").cloned().unwrap_or(json!({}));
                if !state.is_object() {
                    return Err("widget state must be an object".into());
                }
                self.widgets = crate::lua::WidgetManager::new(&paths);
                self.widget_state = state;
                self.overlay = None;
                self.last_render = Instant::now() - Duration::from_secs(1);
                Ok(self.render())
            }
            "widget_state" => {
                let state: Value = field(request, "state")?;
                if !state.is_object() {
                    return Err("widget state must be an object".into());
                }
                self.widget_state = state;
                Ok(json!({"updated":true}))
            }
            "clear_widgets" => {
                self.widgets = crate::lua::WidgetManager::new(&[] as &[&Path]);
                self.overlay = None;
                Ok(json!({"cleared":true}))
            }
            "notify" => {
                self.widget_state["notification"] = json!({"message":field::<String>(request,"message")?,"ttl_ms":field::<u64>(request,"ttl_ms")?});
                Ok(json!({"updated":true}))
            }
            _ => dispatch(&mut self.engine, request),
        }
    }
    fn render(&mut self) -> Value {
        if self.last_render.elapsed() < Duration::from_millis(100) {
            return json!({"diagnostics":[]});
        }
        self.last_render = Instant::now();
        let result = self
            .widgets
            .render(&json!({"engine":self.engine.status(),"state":self.widget_state}));
        let diagnostics: Vec<_> = result
            .diagnostics
            .iter()
            .map(|d| json!({"path":d.path,"message":d.message}))
            .collect();
        if result.spec.status.is_none() && result.spec.elements.is_empty() {
            if let Some(overlay) = self.overlay.as_mut() {
                let _ = overlay.clear();
            }
        } else {
            if self.overlay.is_none() {
                match crate::overlay::Overlay::new(self.engine.profile.target.clone()) {
                    Ok(o) => self.overlay = Some(o),
                    Err(e) => return json!({"diagnostics":diagnostics,"overlay_error":e}),
                }
            }
            if let Some(o) = self.overlay.as_mut() {
                if let Err(e) = o.show(result.spec) {
                    self.overlay = None;
                    return json!({"diagnostics":diagnostics,"overlay_error":e});
                }
            }
        }
        json!({"diagnostics":diagnostics})
    }
}
fn overlay_command(args: &[String]) -> Result<()> {
    let profile: Profile = read_json(arg(args, 1, "profile")?)?;
    let paths: Vec<PathBuf> = args.iter().skip(2).map(PathBuf::from).collect();
    if paths.is_empty() {
        return Err("at least one Lua widget path is required".into());
    }
    let backend = WindowsBackend::new(profile.target.clone())?;
    let engine = Engine::new(backend, profile, state_dir()?)?;
    let mut session = RpcSession::new(engine);
    session.widgets = crate::lua::WidgetManager::new(&paths);
    loop {
        let diagnostics = session.render();
        if diagnostics["diagnostics"]
            .as_array()
            .is_some_and(|x| !x.is_empty())
        {
            eprintln!("{diagnostics}");
        }
        thread::sleep(Duration::from_millis(100));
    }
}
fn tool_catalog() -> Value {
    let runtime = [
        ("status", json!({}), false),
        (
            "input_events",
            json!({"after":{"type":"integer","minimum":0}}),
            false,
        ),
        ("arm", json!({}), true),
        ("pause", json!({}), false),
        ("abort", json!({}), false),
        ("heartbeat", json!({}), false),
        ("tick", json!({}), true),
        ("execute", json!({"actions":"array of Action"}), true),
        ("evaluate", json!({"condition":"Condition"}), false),
        ("pixel", json!({"point":"Point"}), false),
        ("clipboard", json!({}), false),
        (
            "screenshot",
            json!({"region?":"Region","path?":"string"}),
            false,
        ),
        ("ocr", json!({"region?":"Region"}), false),
        ("hotkeys", json!({"enabled":"boolean"}), false),
        ("workflow", json!({"workflow":"Workflow"}), true),
        (
            "widgets",
            json!({"paths":"array of Lua file paths","state?":"JSON"}),
            false,
        ),
        ("widget_state", json!({"state":"JSON"}), false),
        ("clear_widgets", json!({}), false),
        (
            "notify",
            json!({"message":"string","ttl_ms":"integer"}),
            false,
        ),
    ];
    let database = [
        ("execute", json!({"sql":"string","params?":"array"})),
        ("query", json!({"sql":"string","params?":"array"})),
        ("transaction", json!({"statements":"array"})),
        ("migrate", json!({"version":"integer","sql":"string"})),
        (
            "cache_put",
            json!({"namespace":"string","key":"string","value":"JSON","ttl_ms":"integer"}),
        ),
        ("cache_get", json!({"namespace":"string","key":"string"})),
        (
            "cache_invalidate",
            json!({"namespace":"string","key":"string"}),
        ),
        ("cache_clear", json!({"namespace":"string"})),
        ("cache_prune", json!({})),
        ("status", json!({})),
        ("fetch", json!({"request":"FetchRequest"})),
    ];
    json!({"tools":runtime.iter().map(|(op,fields,danger)|json!({"op":op,"target":"rpc","input_schema":tool_schema(op,fields),"input_action":danger})).chain(database.iter().map(|(op,fields)|json!({"op":op,"target":"db","input_schema":tool_schema(op,fields),"input_action":false}))).collect::<Vec<_>>()})
}
fn tool_schema(op: &str, fields: &Value) -> Value {
    let mut properties = serde_json::Map::new();
    properties.insert("op".into(), json!({"const":op,"type":"string"}));
    properties.insert(
        "id".into(),
        json!({"description":"Optional response correlation ID"}),
    );
    let mut required = vec!["op".to_string()];
    if let Some(fields) = fields.as_object() {
        for (name, description) in fields {
            let optional = name.ends_with('?');
            let key = name.trim_end_matches('?');
            let description = description.as_str().unwrap_or("");
            let kind = if description.starts_with("array") {
                "array"
            } else if description == "integer" {
                "integer"
            } else if description == "boolean" {
                "boolean"
            } else if description == "string" || description.contains("paths") {
                "string"
            } else if description == "JSON" {
                "object"
            } else {
                "object"
            };
            let mut spec = if description == "JSON" && key == "value" {
                json!({"description":"JSON value"})
            } else {
                json!({"type":kind,"description":description})
            };
            if kind == "array" {
                spec["items"] = if description.contains("paths") {
                    json!({"type":"string"})
                } else {
                    json!({})
                };
            }
            if description == "FetchRequest" {
                spec = json!({"type":"object","properties":{"url":{"type":"string"},"timeout_ms":{"type":"integer","minimum":1,"maximum":60000},"max_bytes":{"type":"integer","minimum":1,"maximum":8388608},"cache_ttl_ms":{"type":["integer","null"],"minimum":0},"allow_http":{"type":"boolean"}},"required":["url"],"additionalProperties":false});
            }
            properties.insert(key.into(), spec);
            if !optional {
                required.push(key.into())
            }
        }
    }
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fake;
    impl Backend for Fake {
        fn target_present(&mut self) -> Result<bool> {
            Ok(false)
        }
        fn focused(&mut self) -> Result<bool> {
            Ok(false)
        }
        fn key_down(&mut self, _: u16) -> Result<bool> {
            Ok(false)
        }
        fn cursor(&mut self) -> Result<Point> {
            Ok(Point { x: 0.0, y: 0.0 })
        }
        fn pixel(&mut self, _: Point) -> Result<u32> {
            Ok(0)
        }
        fn clipboard(&mut self) -> Result<String> {
            Ok(String::new())
        }
        fn emit(&mut self, _: &InputEvent) -> Result<()> {
            Err("unexpected input".into())
        }
        fn screenshot(&mut self, _: Option<Region>) -> Result<Capture> {
            Err("unexpected capture".into())
        }
        fn ocr(&mut self, _: Option<Region>) -> Result<Vec<TextBox>> {
            Err("unexpected OCR".into())
        }
        fn notify(&mut self, _: &str, _: u64) -> Result<()> {
            Err("unexpected notification".into())
        }
    }
    #[test]
    fn session_controls_and_notification_without_target() {
        let profile = Profile {
            schema_version: 1,
            target: Target {
                window_class: Some("example".into()),
                window_title: None,
            },
            controls: Controls {
                abort: 1,
                reset: 2,
                pause: 3,
                resume: 4,
            },
            tick_ms: 10,
            rules: vec![],
        };
        let dir = std::env::temp_dir().join(format!("rhk-cli-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut session = RpcSession::new(Engine::new(Fake, profile, dir.clone()).unwrap());
        assert_eq!(
            session.dispatch(&json!({"op":"input_events"})).unwrap()["last_sequence"],
            0
        );
        assert!(
            session
                .dispatch(&json!({"op":"input_events","after":-1}))
                .is_err()
        );
        assert_eq!(
            session.dispatch(&json!({"op":"status"})).unwrap()["present"],
            false
        );
        assert_eq!(
            session
                .dispatch(&json!({"op":"hotkeys","enabled":false}))
                .unwrap()["hotkeys_enabled"],
            false
        );
        session
            .dispatch(&json!({"op":"notify","message":"hello","ttl_ms":1000}))
            .unwrap();
        assert_eq!(session.widget_state["notification"]["message"], "hello");
        assert!(
            session
                .dispatch(&json!({"op":"widget_state","state":null}))
                .is_err()
        );
        assert_eq!(
            session.dispatch(&json!({"op":"clear_widgets"})).unwrap()["cleared"],
            true
        );
        let _ = fs::remove_dir_all(dir);
    }
    #[test]
    fn catalog_describes_real_operations() {
        let catalog = tool_catalog();
        let tools = catalog["tools"].as_array().unwrap();
        for (target, op) in [("rpc", "execute"), ("rpc", "widgets"), ("db", "fetch")] {
            let tool = tools
                .iter()
                .find(|t| t["target"] == target && t["op"] == op)
                .unwrap();
            assert_eq!(tool["input_schema"]["type"], "object");
            assert_eq!(tool["input_schema"]["properties"]["op"]["const"], op);
        }
    }
}
