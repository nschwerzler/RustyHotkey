use crate::{Backend, Result, model::*};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const MAX_DEPTH: usize = 64;
const MAX_ITEMS: usize = 10_000;
const MAX_TICK_MS: u64 = 60_000;
const MAX_WAIT_MS: u64 = 86_400_000;

pub fn validate_profile(profile: &Profile) -> Result<()> {
    if profile.schema_version != 1 {
        return Err("schema_version must be 1".into());
    }
    if profile
        .target
        .window_class
        .as_ref()
        .map_or(true, |s| s.trim().is_empty())
        && profile
            .target
            .window_title
            .as_ref()
            .map_or(true, |s| s.trim().is_empty())
    {
        return Err("target requires a window class or title".into());
    }
    for s in [&profile.target.window_class, &profile.target.window_title] {
        if s.as_ref().map_or(false, |x| x.len() > 4096) {
            return Err("target selector is too large".into());
        }
    }
    if profile.tick_ms == 0 || profile.tick_ms > MAX_TICK_MS {
        return Err("tick_ms is outside the supported range".into());
    }
    valid_key(profile.controls.abort)?;
    valid_key(profile.controls.reset)?;
    valid_key(profile.controls.pause)?;
    valid_key(profile.controls.resume)?;
    let controls = [
        profile.controls.abort,
        profile.controls.reset,
        profile.controls.pause,
        profile.controls.resume,
    ];
    for i in 0..controls.len() {
        if controls[i + 1..].contains(&controls[i]) {
            return Err("control keys must be distinct".into());
        }
    }
    if profile.rules.len() > MAX_ITEMS {
        return Err("too many rules".into());
    }
    for rule in &profile.rules {
        if rule.name.trim().is_empty() {
            return Err("rule names must not be empty".into());
        }
        validate_condition(&rule.condition, 0)?;
        validate_actions(&rule.actions, 0)?;
        if rule.cooldown_ms > MAX_WAIT_MS || rule.delay_ms > MAX_WAIT_MS {
            return Err("rule cooldown is too large".into());
        }
    }
    Ok(())
}

fn valid_key(k: u16) -> Result<()> {
    if (1..=0xff).contains(&k) {
        Ok(())
    } else {
        Err("key must be in the 1..=255 range".into())
    }
}
fn valid_button(b: u8) -> Result<()> {
    if (1..=5).contains(&b) {
        Ok(())
    } else {
        Err("button must be in the 1..=5 range".into())
    }
}
fn valid_coord(v: f64) -> Result<()> {
    if v.is_finite() && (0.0..=1.0).contains(&v) {
        Ok(())
    } else {
        Err("coordinates must be finite normalized values in 0..=1".into())
    }
}
fn validate_point(p: Point) -> Result<()> {
    valid_coord(p.x)?;
    valid_coord(p.y)
}
fn validate_region(region: Region) -> Result<()> {
    if !region.x.is_finite()
        || !region.y.is_finite()
        || !region.width.is_finite()
        || !region.height.is_finite()
        || region.x < 0.0
        || region.y < 0.0
        || region.width <= 0.0
        || region.height <= 0.0
        || region.x + region.width > 1.0
        || region.y + region.height > 1.0
    {
        Err("region must be a positive normalized rectangle within 0..=1".into())
    } else {
        Ok(())
    }
}
pub fn validate_condition(c: &Condition, depth: usize) -> Result<()> {
    if depth > MAX_DEPTH {
        return Err("condition nesting is too deep".into());
    }
    match c {
        Condition::Modifiers {
            shift,
            control,
            alt,
            meta,
        } => {
            if shift.is_none() && control.is_none() && alt.is_none() && meta.is_none() {
                return Err("modifier condition requires at least one state".into());
            }
            Ok(())
        }
        Condition::PixelPattern { point, pattern } => {
            validate_point(*point)?;
            pixel_pattern(pattern).map(|_| ())
        }
        Condition::Always | Condition::Focused | Condition::KeyDown { key: _ } => {
            if let Condition::KeyDown { key } = c {
                valid_key(*key)
            } else {
                Ok(())
            }
        }
        Condition::Pixel { point, color, .. } => {
            validate_point(*point)?;
            if *color > 0x00ff_ffff {
                Err("color must be RGB".into())
            } else {
                Ok(())
            }
        }
        Condition::ClipboardContains { text } => {
            if text.len() > 1_000_000 {
                Err("condition text is too large".into())
            } else {
                Ok(())
            }
        }
        Condition::OcrContains { text, region, .. } => {
            if text.trim().is_empty() || text.len() > 4096 {
                return Err("OCR condition text must be non-empty and at most 4096 bytes".into());
            }
            if let Some(region) = region {
                validate_region(*region)?;
            }
            Ok(())
        }
        Condition::All { conditions } | Condition::Any { conditions } => {
            if conditions.len() > MAX_ITEMS {
                return Err("too many conditions".into());
            }
            for x in conditions {
                validate_condition(x, depth + 1)?;
            }
            Ok(())
        }
        Condition::Not { condition } => validate_condition(condition, depth + 1),
    }
}
pub fn validate_action(action: &Action) -> Result<()> {
    validate_actions(std::slice::from_ref(action), 0)
}
pub fn validate_actions(actions: &[Action], depth: usize) -> Result<()> {
    if depth > MAX_DEPTH || actions.len() > MAX_ITEMS {
        return Err("actions exceed supported limits".into());
    }
    for a in actions {
        match a {
            Action::Tap { key } | Action::Key { key, .. } => valid_key(*key)?,
            Action::Move { x, y } => {
                valid_coord(*x)?;
                valid_coord(*y)?;
            }
            Action::Click { button } | Action::Button { button, .. } => valid_button(*button)?,
            Action::ClickBurst {
                button,
                count,
                interval_ms,
                ..
            } => {
                valid_button(*button)?;
                if !(1..=1000).contains(count)
                    || *interval_ms == 0
                    || *interval_ms > MAX_WAIT_MS / u64::from(*count)
                {
                    return Err("click burst count or duration is out of bounds".into());
                }
            }
            Action::Wait { ms } => {
                if *ms > MAX_WAIT_MS {
                    return Err("wait is too large".into());
                }
            }
            Action::Scroll { delta, .. } => {
                if *delta == 0 || delta.unsigned_abs() > 120_000 {
                    return Err("scroll delta is out of bounds".into());
                }
            }
            Action::Pause
            | Action::SetHotkeys { .. }
            | Action::ToggleHotkeys
            | Action::Sound { .. } => {}
            Action::Notify { message, ttl_ms } => {
                if message.len() > 1_000_000 || *ttl_ms > MAX_WAIT_MS {
                    return Err("notification is outside supported limits".into());
                }
            }
            Action::Text { text } => {
                if text.len() > 1_000_000 {
                    return Err("text is too large".into());
                }
            }
        }
    }
    Ok(())
}

#[derive(Clone)]
pub struct ControlHandle {
    abort: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
    heartbeat_ms: Arc<AtomicU64>,
}
impl ControlHandle {
    pub fn request_abort(&self) {
        self.abort.store(true, Ordering::Release);
    }
    pub fn request_pause(&self) {
        self.pause.store(true, Ordering::Release);
    }
    pub fn heartbeat(&self) {
        self.heartbeat_ms.store(now_ms(), Ordering::Release);
    }
}
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis().min(u64::MAX as u128) as u64)
}

pub struct Engine<B: Backend> {
    pub backend: B,
    pub profile: Profile,
    state_dir: PathBuf,
    armed: bool,
    abort_latched: bool,
    paused: bool,
    held_keys: HashSet<u16>,
    held_buttons: HashSet<u8>,
    controls: [bool; 4],
    running: bool,
    last_rules: HashMap<usize, Instant>,
    last_heartbeat: Option<Instant>,
    watchdog: Option<Duration>,
    control: ControlHandle,
    observed_heartbeat: u64,
    hotkeys_enabled: bool,
    dormant: bool,
    pause_marker: Vec<u8>,
    input_events: VecDeque<Value>,
    input_sequence: u64,
    last_tick: Option<Instant>,
    clock_origin: Instant,
    observe_only: bool,
    pending_rules: HashMap<usize, Instant>,
}

impl<B: Backend> Engine<B> {
    pub fn new(mut backend: B, profile: Profile, state_dir: PathBuf) -> Result<Self> {
        validate_profile(&profile)?;
        fs::create_dir_all(&state_dir).map_err(|e| e.to_string())?;
        let latched = state_dir.join("abort.latch").exists();
        let pause_marker = fs::read(state_dir.join("pause.signal")).unwrap_or_default();
        let dormant = !backend.target_present()?;
        let initial_heartbeat = now_ms();
        Ok(Self {
            backend,
            profile,
            state_dir,
            armed: false,
            abort_latched: latched,
            paused: true,
            held_keys: HashSet::new(),
            held_buttons: HashSet::new(),
            controls: [false; 4],
            running: false,
            last_rules: HashMap::new(),
            last_heartbeat: None,
            watchdog: None,
            control: ControlHandle {
                abort: Arc::new(AtomicBool::new(false)),
                pause: Arc::new(AtomicBool::new(false)),
                heartbeat_ms: Arc::new(AtomicU64::new(initial_heartbeat)),
            },
            observed_heartbeat: initial_heartbeat,
            hotkeys_enabled: true,
            dormant,
            pause_marker,
            input_events: VecDeque::with_capacity(512),
            input_sequence: 0,
            last_tick: None,
            clock_origin: Instant::now(),
            observe_only: false,
            pending_rules: HashMap::new(),
        })
    }
    pub fn control_handle(&self) -> ControlHandle {
        self.control.clone()
    }
    pub fn set_observe_only(&mut self, enabled: bool) -> Result<()> {
        self.observe_only = enabled;
        if enabled {
            self.pause()?;
        }
        Ok(())
    }
    fn clock_now(&self) -> Instant {
        self.backend
            .virtual_time_ms()
            .and_then(|ms| self.clock_origin.checked_add(Duration::from_millis(ms)))
            .unwrap_or_else(Instant::now)
    }
    pub fn elapsed_ms(&self) -> u64 {
        self.clock_now()
            .saturating_duration_since(self.clock_origin)
            .as_millis()
            .min(u64::MAX as u128) as u64
    }
    pub fn wait_ms(&mut self, ms: u64) -> Result<()> {
        if ms > MAX_WAIT_MS {
            return Err("wait duration is out of bounds".into());
        }
        let end = self
            .clock_now()
            .checked_add(Duration::from_millis(ms))
            .ok_or("wait clock overflow")?;
        while self.clock_now() < end {
            self.before_input()?;
            let remaining = end
                .saturating_duration_since(self.clock_now())
                .as_millis()
                .min(10) as u64;
            self.backend.sleep_ms(remaining.max(1))?;
        }
        // A boundary observation may have changed focus or requested an abort.
        self.before_input()
    }
    fn latch_path(&self) -> PathBuf {
        self.state_dir.join("abort.latch")
    }
    pub fn latched(&self) -> bool {
        self.abort_latched
    }
    pub fn armed(&self) -> bool {
        self.armed
    }
    pub fn set_hotkeys_enabled(&mut self, enabled: bool) -> Result<()> {
        self.hotkeys_enabled = enabled;
        if !enabled {
            self.release_all()?;
        }
        Ok(())
    }
    fn control_dir(&self) -> PathBuf {
        self.state_dir.join("control")
    }
    fn pid_signal(&self, suffix: &str) -> PathBuf {
        self.control_dir()
            .join(format!("{}.{}", std::process::id(), suffix))
    }
    pub fn exit_requested(&self) -> bool {
        self.state_dir.join("exit.signal").exists() || self.pid_signal("exit").exists()
    }
    pub fn set_watchdog(&mut self, watchdog: Option<Duration>) {
        self.watchdog = watchdog;
        self.heartbeat();
    }
    pub fn heartbeat(&mut self) {
        self.control.heartbeat();
        self.observed_heartbeat = self.control.heartbeat_ms.load(Ordering::Acquire);
        self.last_heartbeat = Some(self.clock_now());
    }
    fn poll_signal(&mut self) -> Result<()> {
        let pause_path = self.state_dir.join("pause.signal");
        if let Ok(marker) = fs::read(&pause_path) {
            if marker.len() <= 128 && marker != self.pause_marker {
                self.pause_marker = marker;
                self.pause()?;
            }
        }
        let pid_pause = self.pid_signal("pause");
        if pid_pause.exists() {
            self.pause()?;
            let _ = fs::remove_file(pid_pause);
        }
        let hotkey_path = self.pid_signal("hotkeys");
        if let Ok(data) = fs::read(&hotkey_path) {
            if data.len() <= 256 {
                if let Ok(v) = serde_json::from_slice::<Value>(&data) {
                    if let Some(enabled) = v.get("enabled").and_then(Value::as_bool) {
                        self.set_hotkeys_enabled(enabled)?;
                    }
                }
            }
            let _ = fs::remove_file(hotkey_path);
        }
        if self.control.abort.swap(false, Ordering::AcqRel) {
            self.abort()?;
        }
        if self.control.pause.swap(false, Ordering::AcqRel) {
            self.pause()?;
        }
        let beat = self.control.heartbeat_ms.load(Ordering::Acquire);
        if beat != 0 && beat != self.observed_heartbeat {
            self.observed_heartbeat = beat;
            self.last_heartbeat = self
                .clock_now()
                .checked_sub(Duration::from_millis(now_ms().saturating_sub(beat)));
        }
        Ok(())
    }
    pub fn safety_check(&mut self) -> Result<()> {
        self.poll_signal()?;
        validate_profile(&self.profile)?;
        if self.abort_latched {
            return Err("abort latch is set".into());
        }
        if !self.backend.focused()? {
            self.armed = false;
            self.paused = true;
            self.pending_rules.clear();
            let _ = self.release_all();
            return Err("target is not focused".into());
        }
        Ok(())
    }
    pub fn arm(&mut self) -> Result<()> {
        if self.observe_only {
            return Err("session is observe-only".into());
        }
        self.safety_check()?;
        self.armed = true;
        self.paused = false;
        self.heartbeat();
        Ok(())
    }
    fn release_all(&mut self) -> Result<()> {
        let keys: Vec<_> = self.held_keys.iter().copied().collect();
        let buttons: Vec<_> = self.held_buttons.iter().copied().collect();
        let mut first = None;
        for key in keys {
            if let Err(e) = self.emit_input(&InputEvent::Key { key, down: false }) {
                if first.is_none() {
                    first = Some(e);
                }
            } else {
                self.held_keys.remove(&key);
            }
        }
        for button in buttons {
            if let Err(e) = self.emit_input(&InputEvent::Button {
                button,
                down: false,
            }) {
                if first.is_none() {
                    first = Some(e);
                }
            } else {
                self.held_buttons.remove(&button);
            }
        }
        first.map_or(Ok(()), Err)
    }
    fn record_event(&mut self, kind: &str, key: Option<u16>) {
        self.input_sequence = self.input_sequence.wrapping_add(1);
        let at_ms = self.backend.virtual_time_ms().unwrap_or_else(now_ms);
        let mut event = json!({"sequence": self.input_sequence, "kind": kind, "at_ms": at_ms});
        if let Some(key) = key {
            event["key"] = Value::String(key_name(key));
        }
        if self.input_events.len() == 512 {
            self.input_events.pop_front();
        }
        self.input_events.push_back(event);
    }
    fn emit_input(&mut self, event: &InputEvent) -> Result<()> {
        self.backend.emit(event)?;
        match event {
            InputEvent::Key { key, down } => {
                self.record_event(if *down { "down" } else { "up" }, Some(*key))
            }
            InputEvent::Button { button, down } => self.record_event(
                if *down { "down" } else { "up" },
                Some(0x100 + *button as u16),
            ),
            _ => {}
        }
        Ok(())
    }
    pub fn input_events(&self, after: u64) -> Value {
        let events: Vec<_> = self
            .input_events
            .iter()
            .filter(|e| {
                e.get("sequence")
                    .and_then(Value::as_u64)
                    .map_or(false, |n| n > after)
            })
            .cloned()
            .collect();
        let oldest = self
            .input_events
            .front()
            .and_then(|e| e.get("sequence"))
            .and_then(Value::as_u64)
            .unwrap_or(self.input_sequence.saturating_add(1));
        let mut held: Vec<_> = self.held_keys.iter().map(|k| key_name(*k)).collect();
        held.extend(
            self.held_buttons
                .iter()
                .map(|b| key_name(0x100 + *b as u16)),
        );
        json!({"events": events, "last_sequence": self.input_sequence, "oldest_sequence": oldest, "truncated": after.saturating_add(1) < oldest || after > self.input_sequence, "held": held, "limit": 512})
    }
    pub fn abort(&mut self) -> Result<()> {
        self.pending_rules.clear();
        self.abort_latched = true;
        self.armed = false;
        self.paused = true;
        self.pending_rules.clear();
        let write = fs::write(self.latch_path(), b"abort").map_err(|e| e.to_string());
        let release = self.release_all();
        write.and(release)
    }
    pub fn pause(&mut self) -> Result<()> {
        self.pending_rules.clear();
        self.paused = true;
        self.pending_rules.clear();
        self.armed = false;
        self.release_all()
    }
    fn reset_edge(&mut self) -> Result<()> {
        if !self.abort_latched || !self.backend.focused()? {
            return Ok(());
        }
        self.abort_latched = false;
        let _ = fs::remove_file(self.latch_path());
        Ok(())
    }
    pub fn poll_controls(&mut self) -> Result<()> {
        self.poll_signal()?;
        self.dormant = !self.backend.target_present()?;
        if self.dormant {
            self.armed = false;
            self.paused = true;
            self.pending_rules.clear();
            self.release_all()?;
        }
        let in_target = !self.dormant && self.backend.focused()?;
        let next = [
            self.backend.key_down(self.profile.controls.abort)?,
            self.backend.key_down(self.profile.controls.reset)?,
            self.backend.key_down(self.profile.controls.pause)?,
            self.backend.key_down(self.profile.controls.resume)?,
        ];
        let old = self.controls;
        self.controls = next;
        if next[0] && !old[0] {
            self.abort()?;
        }
        if next[1] && !old[1] {
            self.reset_edge()?;
        }
        if self.hotkeys_enabled && in_target && next[2] && !old[2] {
            self.pause()?;
        }
        if self.hotkeys_enabled && in_target && next[3] && !old[3] && !self.abort_latched {
            self.arm()?;
        }
        Ok(())
    }
    fn check_input(&mut self) -> Result<()> {
        if self.observe_only {
            return Err("session is observe-only".into());
        }
        if self.abort_latched || !self.armed || self.paused {
            return Err("engine is not ready".into());
        }
        if !self.backend.target_present()? {
            self.armed = false;
            self.paused = true;
            self.pending_rules.clear();
            self.dormant = true;
            let _ = self.release_all();
            return Err("target absent".into());
        }
        self.dormant = false;
        if !self.backend.focused()? {
            self.armed = false;
            self.paused = true;
            self.pending_rules.clear();
            let _ = self.release_all();
            return Err("target lost focus".into());
        }
        Ok(())
    }
    fn before_input(&mut self) -> Result<()> {
        self.poll_controls()?;
        if let Some(max) = self.watchdog {
            if self.last_heartbeat.map_or(true, |t| {
                self.clock_now().saturating_duration_since(t) > max
            }) {
                self.paused = true;
                self.pending_rules.clear();
                self.armed = false;
                let _ = self.release_all();
                return Err("heartbeat watchdog expired".into());
            }
        }
        self.check_input()
    }
    pub fn execute(&mut self, actions: &[Action]) -> Result<()> {
        validate_actions(actions, 0)?;
        let c = [
            self.profile.controls.abort,
            self.profile.controls.reset,
            self.profile.controls.pause,
            self.profile.controls.resume,
        ];
        for a in actions {
            match a {
                Action::Tap { key } | Action::Key { key, .. } if c.contains(key) => {
                    return Err("control keys are reserved".into());
                }
                Action::Click { button } | Action::Button { button, .. } if *button == 0 => {
                    return Err("invalid button".into());
                }
                _ => {}
            }
        }
        if self.running {
            return Err("execution already in progress".into());
        }
        self.running = true;
        let result = self.execute_inner(actions);
        self.running = false;
        if result.is_err() {
            self.armed = false;
            self.paused = true;
            self.pending_rules.clear();
            let _ = self.release_all();
        }
        result
    }
    fn execute_inner(&mut self, actions: &[Action]) -> Result<()> {
        for action in actions {
            self.before_input()?;
            match action {
                Action::Tap { key } => {
                    self.emit_input(&InputEvent::Key {
                        key: *key,
                        down: true,
                    })?;
                    self.held_keys.insert(*key);
                    self.before_input()?;
                    self.emit_input(&InputEvent::Key {
                        key: *key,
                        down: false,
                    })?;
                    self.held_keys.remove(key);
                }
                Action::Key { key, down } => {
                    self.emit_input(&InputEvent::Key {
                        key: *key,
                        down: *down,
                    })?;
                    if *down {
                        self.held_keys.insert(*key);
                    } else {
                        self.held_keys.remove(key);
                    }
                }
                Action::Move { x, y } => self.emit_input(&InputEvent::Move { x: *x, y: *y })?,
                Action::Click { button } => {
                    self.emit_input(&InputEvent::Button {
                        button: *button,
                        down: true,
                    })?;
                    self.held_buttons.insert(*button);
                    self.before_input()?;
                    self.emit_input(&InputEvent::Button {
                        button: *button,
                        down: false,
                    })?;
                    self.held_buttons.remove(button);
                }
                Action::ClickBurst {
                    button,
                    count,
                    interval_ms,
                    lock_position,
                } => {
                    let point = if *lock_position {
                        Some(self.backend.cursor()?)
                    } else {
                        None
                    };
                    if let Some(point) = point {
                        validate_point(point)?;
                    }
                    for _ in 0..*count {
                        self.wait_ms(*interval_ms)?;
                        if let Some(point) = point {
                            self.execute_inner(&[Action::Move {
                                x: point.x,
                                y: point.y,
                            }])?;
                        }
                        self.execute_inner(&[Action::Click { button: *button }])?;
                    }
                }
                Action::Button { button, down } => {
                    self.emit_input(&InputEvent::Button {
                        button: *button,
                        down: *down,
                    })?;
                    if *down {
                        self.held_buttons.insert(*button);
                    } else {
                        self.held_buttons.remove(button);
                    }
                }
                Action::Text { text } => self
                    .backend
                    .emit(&InputEvent::Text { text: text.clone() })?,
                Action::Scroll { delta, horizontal } => self.emit_input(&InputEvent::Wheel {
                    delta: *delta,
                    horizontal: *horizontal,
                })?,
                Action::Pause => self.pause()?,
                Action::SetHotkeys { enabled } => self.set_hotkeys_enabled(*enabled)?,
                Action::ToggleHotkeys => self.set_hotkeys_enabled(!self.hotkeys_enabled)?,
                Action::Sound { cue } => self.backend.sound(*cue)?,
                Action::Notify { message, ttl_ms } => self.backend.notify(message, *ttl_ms)?,
                Action::Wait { ms } => {
                    self.wait_ms(*ms)?;
                }
            }
        }
        Ok(())
    }
    pub fn matches(&mut self, condition: &Condition) -> Result<bool> {
        validate_condition(condition, 0)?;
        Ok(match condition {
            Condition::Always => true,
            Condition::PixelPattern { point, pattern } => {
                let (color, mask) = pixel_pattern(pattern)?;
                self.backend.pixel(*point)? & mask == color
            }
            Condition::Focused => self.backend.focused()?,
            Condition::Modifiers {
                shift,
                control,
                alt,
                meta,
            } => {
                let mut matched = true;
                for (desired, key) in [(shift, 16), (control, 17), (alt, 18)] {
                    if let Some(desired) = desired {
                        matched &= self.backend.key_down(key)? == *desired;
                    }
                }
                if let Some(desired) = meta {
                    matched &=
                        (self.backend.key_down(91)? || self.backend.key_down(92)?) == *desired;
                }
                matched
            }
            Condition::KeyDown { key } => self.backend.key_down(*key)?,
            Condition::Pixel {
                point,
                color,
                tolerance,
            } => {
                let got = self.backend.pixel(*point)?;
                color_distance(got, *color) <= *tolerance as u32
            }
            Condition::ClipboardContains { text } => self.backend.clipboard()?.contains(text),
            Condition::OcrContains {
                text,
                region,
                case_sensitive,
            } => {
                let boxes = self.backend.ocr(*region)?;
                let observed = boxes
                    .iter()
                    .map(|word| word.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                if *case_sensitive {
                    observed.contains(text)
                } else {
                    observed.to_lowercase().contains(&text.to_lowercase())
                }
            }
            Condition::All { conditions } => {
                for c in conditions {
                    if !self.matches(c)? {
                        return Ok(false);
                    }
                }
                true
            }
            Condition::Any { conditions } => {
                for c in conditions {
                    if self.matches(c)? {
                        return Ok(true);
                    }
                }
                false
            }
            Condition::Not { condition } => !self.matches(condition)?,
        })
    }
    pub fn tick(&mut self) -> Result<()> {
        let result = self.tick_inner();
        if result.is_err() {
            self.armed = false;
            self.paused = true;
            self.pending_rules.clear();
            let _ = self.release_all();
        }
        result
    }
    fn tick_inner(&mut self) -> Result<()> {
        self.poll_controls()?;
        if !self.backend.target_present()? {
            self.armed = false;
            self.paused = true;
            self.pending_rules.clear();
            self.dormant = true;
            let _ = self.release_all();
            return Ok(());
        }
        self.dormant = false;
        if !self.armed {
            return Ok(());
        }
        if let Some(max) = self.watchdog {
            if self.last_heartbeat.map_or(true, |t| {
                self.clock_now().saturating_duration_since(t) > max
            }) {
                self.pause()?;
                return Err("heartbeat watchdog expired".into());
            }
        }
        self.check_input()?;
        let now = self.clock_now();
        self.last_tick = Some(now);
        for i in 0..self.profile.rules.len() {
            let (enabled, cooldown, delay, condition, actions) = {
                let r = &self.profile.rules[i];
                (
                    r.enabled,
                    r.cooldown_ms,
                    r.delay_ms,
                    r.condition.clone(),
                    r.actions.clone(),
                )
            };
            if !enabled
                || (!self.hotkeys_enabled && contains_key_condition(&condition))
                || self.last_rules.get(&i).map_or(false, |t| {
                    self.clock_now().saturating_duration_since(*t) < Duration::from_millis(cooldown)
                })
            {
                self.pending_rules.remove(&i);
                continue;
            }
            if !self.matches(&condition)? {
                self.pending_rules.remove(&i);
                continue;
            }
            if delay > 0 {
                let clock = self.clock_now();
                let deadline = *self
                    .pending_rules
                    .entry(i)
                    .or_insert(clock + Duration::from_millis(delay));
                if clock < deadline {
                    continue;
                }
            }
            {
                self.pending_rules.remove(&i);
                self.execute(&actions)?;
                self.last_rules.insert(i, self.clock_now());
            }
        }
        Ok(())
    }
    pub fn status(&self) -> Value {
        let now = self.clock_now();
        let pending: Vec<_> = self.pending_rules.iter().filter_map(|(index,deadline)| self.profile.rules.get(*index).map(|rule| json!({"name":rule.name,"remaining_ms":deadline.saturating_duration_since(now).as_millis()}))).collect();
        let rules: Vec<_> = self.profile.rules.iter().enumerate().map(|(index,rule)| json!({"name":rule.name,"enabled":rule.enabled,"cooldown_remaining_ms":self.last_rules.get(&index).map_or(0,|last| rule.cooldown_ms.saturating_sub(now.saturating_duration_since(*last).as_millis().min(u64::MAX as u128) as u64))})).collect();
        json!({"pending_rules":pending,"rules":rules,"observe_only": self.observe_only, "armed": self.armed, "paused": self.paused, "abort_latched": self.abort_latched, "held_keys": self.held_keys.len(), "held_buttons": self.held_buttons.len(), "running": true, "executing": self.running, "hotkeys_enabled": self.hotkeys_enabled && !self.dormant && !self.observe_only, "requested_hotkeys": self.hotkeys_enabled, "dormant": self.dormant})
    }
}
fn pixel_pattern(pattern: &str) -> Result<(u32, u32)> {
    let value = pattern.strip_prefix('#').unwrap_or(pattern);
    if value.len() != 6 {
        return Err("pixel pattern must contain six hex digits or wildcards".into());
    }
    let mut color = 0u32;
    let mut mask = 0u32;
    for digit in value.chars() {
        color <<= 4;
        mask <<= 4;
        if matches!(digit, '*' | '?') {
            continue;
        }
        color |= digit
            .to_digit(16)
            .ok_or("pixel pattern contains an invalid digit")?;
        mask |= 15;
    }
    Ok((color, mask))
}
fn contains_key_condition(c: &Condition) -> bool {
    match c {
        Condition::KeyDown { .. } => true,
        Condition::All { conditions } | Condition::Any { conditions } => {
            conditions.iter().any(contains_key_condition)
        }
        Condition::Not { condition } => contains_key_condition(condition),
        _ => false,
    }
}
fn key_name(key: u16) -> String {
    if (0x101..=0x105).contains(&key) {
        return format!("Mouse{}", key - 0x100);
    }
    if (0x60..=0x69).contains(&key) {
        return format!("Numpad{}", key - 0x60);
    }
    match key {
        0x41..=0x5a => format!("Key{}", char::from_u32(key as u32).unwrap_or('?')),
        0x30..=0x39 => format!("Digit{}", key - 0x30),
        0x70..=0x87 => format!("F{}", key - 0x6f),
        0x08 => "Backspace".into(),
        0x13 => "Pause".into(),
        0x14 => "CapsLock".into(),
        0x10 => "ShiftLeft".into(),
        0x11 => "ControlLeft".into(),
        0x12 => "AltLeft".into(),
        0x20 => "Space".into(),
        0x1b => "Escape".into(),
        0x0d => "Enter".into(),
        0x09 => "Tab".into(),
        0x25 => "ArrowLeft".into(),
        0x26 => "ArrowUp".into(),
        0x27 => "ArrowRight".into(),
        0x28 => "ArrowDown".into(),
        0x21 => "PageUp".into(),
        0x22 => "PageDown".into(),
        0x23 => "End".into(),
        0x24 => "Home".into(),
        0x2c => "PrintScreen".into(),
        0x2d => "Insert".into(),
        0x2e => "Delete".into(),
        0x5b => "MetaLeft".into(),
        0x5c => "MetaRight".into(),
        0x5d => "ContextMenu".into(),
        0x6a => "NumpadMultiply".into(),
        0x6b => "NumpadAdd".into(),
        0x6d => "NumpadSubtract".into(),
        0x6e => "NumpadDecimal".into(),
        0x6f => "NumpadDivide".into(),
        0x90 => "NumLock".into(),
        0x91 => "ScrollLock".into(),
        0xa0 => "ShiftLeft".into(),
        0xa1 => "ShiftRight".into(),
        0xa2 => "ControlLeft".into(),
        0xa3 => "ControlRight".into(),
        0xa4 => "AltLeft".into(),
        0xa5 => "AltRight".into(),
        0xba => "Semicolon".into(),
        0xbb => "Equal".into(),
        0xbc => "Comma".into(),
        0xbd => "Minus".into(),
        0xbe => "Period".into(),
        0xbf => "Slash".into(),
        0xc0 => "Backquote".into(),
        0xdb => "BracketLeft".into(),
        0xdc => "Backslash".into(),
        0xdd => "BracketRight".into(),
        0xde => "Quote".into(),
        _ => format!("VK_{:02X}", key),
    }
}
fn color_distance(a: u32, b: u32) -> u32 {
    let ar = (a >> 16) & 255;
    let ag = (a >> 8) & 255;
    let ab = a & 255;
    let br = (b >> 16) & 255;
    let bg = (b >> 8) & 255;
    let bb = b & 255;
    ar.abs_diff(br).max(ag.abs_diff(bg)).max(ab.abs_diff(bb))
}
impl<B: Backend> Drop for Engine<B> {
    fn drop(&mut self) {
        let _ = self.release_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    struct Fake {
        focused: bool,
        keys: HashSet<u16>,
        events: Arc<Mutex<Vec<InputEvent>>>,
    }
    impl Fake {
        fn new(events: Arc<Mutex<Vec<InputEvent>>>) -> Self {
            Self {
                focused: true,
                keys: HashSet::new(),
                events,
            }
        }
    }
    impl Backend for Fake {
        fn focused(&mut self) -> Result<bool> {
            Ok(self.focused)
        }
        fn key_down(&mut self, key: u16) -> Result<bool> {
            Ok(self.keys.contains(&key))
        }
        fn cursor(&mut self) -> Result<Point> {
            Ok(Point { x: 0.5, y: 0.5 })
        }
        fn pixel(&mut self, _: Point) -> Result<u32> {
            Ok(0)
        }
        fn clipboard(&mut self) -> Result<String> {
            Ok(String::new())
        }
        fn emit(&mut self, event: &InputEvent) -> Result<()> {
            self.events.lock().unwrap().push(event.clone());
            Ok(())
        }
        fn screenshot(&mut self, _: Option<Region>) -> Result<Capture> {
            Ok(Capture {
                width: 0,
                height: 0,
                bgra: vec![],
            })
        }
        fn ocr(&mut self, _: Option<Region>) -> Result<Vec<TextBox>> {
            Ok(vec![])
        }
        fn notify(&mut self, _: &str, _: u64) -> Result<()> {
            Ok(())
        }
    }
    fn profile() -> Profile {
        Profile {
            schema_version: 1,
            target: Target {
                window_class: Some("generic".into()),
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
        }
    }
    #[test]
    fn native_key_names_match_input_monitor_dom_codes() {
        let mappings = [
            (8, "Backspace"),
            (9, "Tab"),
            (13, "Enter"),
            (16, "ShiftLeft"),
            (17, "ControlLeft"),
            (18, "AltLeft"),
            (19, "Pause"),
            (20, "CapsLock"),
            (27, "Escape"),
            (32, "Space"),
            (33, "PageUp"),
            (34, "PageDown"),
            (35, "End"),
            (36, "Home"),
            (37, "ArrowLeft"),
            (38, "ArrowUp"),
            (39, "ArrowRight"),
            (40, "ArrowDown"),
            (44, "PrintScreen"),
            (45, "Insert"),
            (46, "Delete"),
            (91, "MetaLeft"),
            (92, "MetaRight"),
            (93, "ContextMenu"),
            (96, "Numpad0"),
            (105, "Numpad9"),
            (106, "NumpadMultiply"),
            (107, "NumpadAdd"),
            (109, "NumpadSubtract"),
            (110, "NumpadDecimal"),
            (111, "NumpadDivide"),
            (144, "NumLock"),
            (145, "ScrollLock"),
            (160, "ShiftLeft"),
            (161, "ShiftRight"),
            (162, "ControlLeft"),
            (163, "ControlRight"),
            (164, "AltLeft"),
            (165, "AltRight"),
            (186, "Semicolon"),
            (222, "Quote"),
        ];
        for (key, expected) in mappings {
            assert_eq!(key_name(key), expected, "virtual key {key}");
        }
        assert_eq!(key_name(112), "F1");
        assert_eq!(key_name(135), "F24");
        assert_eq!(key_name(0x100), "VK_100");
        assert_eq!(key_name(0x106), "VK_106");
        assert_eq!(key_name(0xdead), "VK_DEAD");
    }
    #[test]
    fn starts_paused_and_latches_abort() {
        let dir = std::env::temp_dir().join(format!("rustyhotkey-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let events = Arc::new(Mutex::new(vec![]));
        let mut e = Engine::new(Fake::new(events), profile(), dir.clone()).unwrap();
        assert!(!e.armed());
        assert!(!e.latched());
        e.abort().unwrap();
        assert!(e.latched());
        assert!(dir.join("abort.latch").exists());
        let _ = fs::remove_dir_all(dir);
    }
    #[test]
    fn rejects_bad_profile_and_reserved_key() {
        let mut p = profile();
        p.schema_version = 2;
        assert!(validate_profile(&p).is_err());
        p.schema_version = 1;
        let events = Arc::new(Mutex::new(vec![]));
        let mut e = Engine::new(
            Fake::new(events),
            p,
            std::env::temp_dir().join("rustyhotkey-validation"),
        )
        .unwrap();
        e.arm().unwrap();
        assert!(e.execute(&[Action::Tap { key: 1 }]).is_err());
    }
    #[test]
    fn cooldown_does_not_fire_twice() {
        let events = Arc::new(Mutex::new(vec![]));
        let mut p = profile();
        p.rules.push(Rule {
            name: "once".into(),
            condition: Condition::Always,
            actions: vec![Action::Tap { key: 10 }],
            cooldown_ms: 10_000,
            delay_ms: 0,
            enabled: true,
        });
        let mut e = Engine::new(
            Fake::new(events.clone()),
            p,
            std::env::temp_dir().join("rustyhotkey-cooldown"),
        )
        .unwrap();
        e.arm().unwrap();
        e.tick().unwrap();
        e.tick().unwrap();
        assert_eq!(events.lock().unwrap().len(), 2);
    }
    #[test]
    fn shared_abort_interrupts_wait() {
        let dir = std::env::temp_dir().join(format!("rustyhotkey-control-{}", now_ms()));
        let _ = fs::remove_dir_all(&dir);
        let events = Arc::new(Mutex::new(vec![]));
        let mut e = Engine::new(Fake::new(events), profile(), dir).unwrap();
        e.arm().unwrap();
        let h = e.control_handle();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(15));
            h.request_abort();
        });
        let started = Instant::now();
        assert!(e.execute(&[Action::Wait { ms: 500 }]).is_err());
        assert!(started.elapsed() < Duration::from_millis(100));
        assert!(e.latched());
    }
    #[test]
    fn stale_watchdog_releases_and_disarms() {
        let events = Arc::new(Mutex::new(vec![]));
        let mut e = Engine::new(
            Fake::new(events),
            profile(),
            std::env::temp_dir().join("rustyhotkey-watchdog"),
        )
        .unwrap();
        e.arm().unwrap();
        e.set_watchdog(Some(Duration::from_millis(1)));
        std::thread::sleep(Duration::from_millis(5));
        assert!(
            e.execute(&[Action::Key {
                key: 10,
                down: true
            }])
            .is_err()
        );
        assert!(!e.armed());
        assert!(e.held_keys.is_empty());
    }
    #[test]
    fn input_telemetry_records_successful_keys_without_text() {
        let events = Arc::new(Mutex::new(vec![]));
        let dir = std::env::temp_dir().join(format!("rustyhotkey-events-{}", now_ms()));
        let _ = fs::remove_dir_all(&dir);
        let mut e = Engine::new(Fake::new(events), profile(), dir).unwrap();
        e.arm().unwrap();
        e.execute(&[
            Action::Tap { key: 0x41 },
            Action::Text {
                text: "secret".into(),
            },
        ])
        .unwrap();
        let snapshot = e.input_events(0);
        let list = snapshot["events"].as_array().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0]["key"], "KeyA");
        assert!(snapshot.to_string().find("secret").is_none());
    }
    #[test]
    fn input_cursor_reports_overflow_and_held_mouse() {
        let events = Arc::new(Mutex::new(vec![]));
        let dir = std::env::temp_dir().join(format!("rustyhotkey-ring-{}", now_ms()));
        let _ = fs::remove_dir_all(&dir);
        let mut e = Engine::new(Fake::new(events), profile(), dir).unwrap();
        e.arm().unwrap();
        e.execute(&[Action::Button {
            button: 1,
            down: true,
        }])
        .unwrap();
        for _ in 0..260 {
            e.execute(&[Action::Tap { key: 0x41 }]).unwrap();
        }
        let snapshot = e.input_events(0);
        assert!(snapshot["truncated"].as_bool().unwrap());
        assert!(snapshot["oldest_sequence"].as_u64().unwrap() > 1);
        assert!(
            snapshot["held"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "Mouse1")
        );
    }
    #[test]
    fn pause_releases_owned_keyboard_and_mouse_inputs() {
        let events = Arc::new(Mutex::new(vec![]));
        let dir = std::env::temp_dir().join(format!("rustyhotkey-release-{}", now_ms()));
        let _ = fs::remove_dir_all(&dir);
        let mut e = Engine::new(Fake::new(events.clone()), profile(), dir).unwrap();
        e.arm().unwrap();
        e.execute(&[
            Action::Key {
                key: 0x41,
                down: true,
            },
            Action::Button {
                button: 1,
                down: true,
            },
        ])
        .unwrap();
        e.pause().unwrap();
        let snapshot = e.input_events(0);
        assert!(snapshot["held"].as_array().unwrap().is_empty());
        assert!(
            snapshot["events"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v["kind"] == "up")
        );
    }
    #[test]
    fn observe_only_refuses_arm_and_hotkey_disable_releases() {
        let events = Arc::new(Mutex::new(vec![]));
        let dir = std::env::temp_dir().join(format!("rhk-observe-{}", now_ms()));
        let mut engine = Engine::new(Fake::new(events.clone()), profile(), dir.clone()).unwrap();
        engine.set_observe_only(true).unwrap();
        assert!(engine.arm().is_err());
        assert!(engine.execute(&[Action::Tap { key: 65 }]).is_err());
        assert!(events.lock().unwrap().is_empty());
        engine.set_observe_only(false).unwrap();
        engine.arm().unwrap();
        engine
            .execute(&[
                Action::Key {
                    key: 65,
                    down: true,
                },
                Action::Button {
                    button: 1,
                    down: true,
                },
            ])
            .unwrap();
        engine.set_hotkeys_enabled(false).unwrap();
        assert!(
            engine.input_events(0)["held"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(engine.status()["hotkeys_enabled"], false);
        drop(engine);
        let _ = fs::remove_dir_all(dir);
    }
}
