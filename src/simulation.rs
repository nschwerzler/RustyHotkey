//! Bounded, deterministic what-if execution against the real engine and a
//! scripted backend. This backend records output and never emits OS input.

use crate::{Backend, Result, model::*, runtime::Engine};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

const MAX_FRAMES: usize = 10_000;
const MAX_OPERATIONS: usize = 10_000;
const MAX_KEYS: usize = 256;
const MAX_PIXELS: usize = 10_000;
const MAX_OCR_BOXES: usize = 10_000;
const MAX_OCR_TEXT_BYTES: usize = 4096;
const MAX_TRACE_EVENTS: usize = 100_000;
const MAX_TRACE_BYTES: usize = 1024 * 1024;
const MAX_EFFECTS: usize = 10_000;
const MAX_EFFECT_BYTES: usize = 1024 * 1024;
const MAX_RESULT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SimulationScenario {
    pub schema_version: u32,
    pub profile: Profile,
    pub frames: Vec<SimulationFrame>,
}

/// Each frame atomically replaces all observations before running its steps.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SimulationFrame {
    /// Virtual scenario time in milliseconds. Must increase strictly and stay
    /// within one day; waits can apply later observations while they advance.
    pub at_ms: u64,
    pub observation: Observation,
    #[serde(default)]
    pub operations: Vec<SimulationOperation>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub target_present: bool,
    pub focused: bool,
    #[serde(default)]
    pub keys_down: Vec<u16>,
    #[serde(default)]
    pub pixels: Vec<PixelObservation>,
    #[serde(default)]
    pub clipboard: String,
    #[serde(default)]
    pub ocr: Vec<TextBox>,
    #[serde(default)]
    pub cursor: Point,
}

/// Backward-compatible descriptive name for a scripted observation frame.
pub type SimulationObservation = Observation;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PixelObservation {
    pub point: Point,
    pub color: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum SimulationOperation {
    Arm,
    Tick,
    Execute {
        actions: Vec<Action>,
    },
    Pause,
    Abort,
    Heartbeat,
    Watchdog {
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    AssertCondition {
        condition: Condition,
        expected: bool,
    },
    AssertState {
        expected: StateExpectation,
    },
}

/// Fields omitted from an expectation are not checked.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateExpectation {
    pub armed: Option<bool>,
    pub paused: Option<bool>,
    pub abort_latched: Option<bool>,
    pub dormant: Option<bool>,
    pub hotkeys_enabled: Option<bool>,
    pub held_keys: Option<usize>,
    pub held_buttons: Option<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SimulationResult {
    pub schema_version: u32,
    pub frames: Vec<FrameResult>,
    pub events: Vec<ObservedEvent>,
    pub rule_effects: Vec<RuleEffect>,
    pub notifications: Vec<SimulatedNotification>,
    pub sounds: Vec<SoundCue>,
    pub final_status: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SimulatedNotification {
    pub message: String,
    pub ttl_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameResult {
    pub frame: usize,
    pub operations: Vec<OperationResult>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationResult {
    pub operation: usize,
    pub op: String,
    pub ok: bool,
    pub error: Option<String>,
    pub emitted_events: Vec<InputEvent>,
    pub notifications: Vec<SimulatedNotification>,
    pub sounds: Vec<SoundCue>,
    pub status: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedEvent {
    pub frame: usize,
    pub operation: usize,
    pub at_ms: u64,
    pub event: InputEvent,
}

/// Events emitted while a `tick` runs are the externally observable rule
/// effects. This deliberately reports effects, rather than inferring rule
/// matches that the engine does not expose.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleEffect {
    pub frame: usize,
    pub operation: usize,
    pub events: Vec<InputEvent>,
    pub notifications: Vec<SimulatedNotification>,
    pub sounds: Vec<SoundCue>,
    pub status: Value,
}

#[derive(Clone, Default)]
struct ScriptedState {
    observation: Option<Observation>,
    timeline: Vec<(u64, Observation)>,
    virtual_time_ms: u64,
    events: Vec<TimedInputEvent>,
    trace_bytes: usize,
    notifications: Vec<(String, u64)>,
    effect_bytes: usize,
    sounds: Vec<SoundCue>,
}

#[derive(Clone)]
struct TimedInputEvent {
    at_ms: u64,
    event: InputEvent,
}

/// Run the supplied bounded scenario using the real Engine and this
/// non-native backend. `state_dir` is used by Engine for its abort latch and
/// must be a caller-owned scenario directory outside the repository.
pub fn run_scenario(scenario: SimulationScenario, state_dir: &Path) -> Result<SimulationResult> {
    validate_scenario(&scenario)?;
    let frame_count = scenario.frames.len();
    let state = Arc::new(Mutex::new(ScriptedState::default()));
    {
        let mut scripted = state.lock().map_err(|_| "simulation state lock poisoned")?;
        scripted.timeline = scenario
            .frames
            .iter()
            .map(|frame| (frame.at_ms, frame.observation.clone()))
            .collect();
        apply_timeline(&mut scripted, 0);
    }
    let mut engine = Engine::new(
        SimulationBackend {
            state: state.clone(),
        },
        scenario.profile,
        state_dir.to_path_buf(),
    )?;
    let mut frame_results = Vec::with_capacity(scenario.frames.len());
    let mut all_events = Vec::new();
    let mut rule_effects = Vec::new();

    for (frame_index, frame) in scenario.frames.into_iter().enumerate() {
        set_virtual_time(&state, frame.at_ms)?;
        let mut operations = Vec::with_capacity(frame.operations.len());
        for (operation_index, operation) in frame.operations.into_iter().enumerate() {
            let before = event_count(&state)?;
            let (notification_start, sound_start) = effect_counts(&state)?;
            let op_name = operation.name().to_string();
            let result = run_operation(&mut engine, &operation);
            let timed_events = events_since(&state, before)?;
            let emitted_events: Vec<_> = timed_events
                .iter()
                .map(|event| event.event.clone())
                .collect();
            let (notifications, sounds) = effects_since(&state, notification_start, sound_start)?;
            let status = engine.status();
            let (ok, error) = match result {
                Ok(()) => (true, None),
                Err(error) => (false, Some(error)),
            };
            for event in &timed_events {
                all_events.push(ObservedEvent {
                    frame: frame_index,
                    operation: operation_index,
                    at_ms: event.at_ms,
                    event: event.event.clone(),
                });
            }
            if op_name == "tick" {
                rule_effects.push(RuleEffect {
                    frame: frame_index,
                    operation: operation_index,
                    events: emitted_events.clone(),
                    notifications: notifications.clone(),
                    sounds: sounds.clone(),
                    status: status.clone(),
                });
            }
            operations.push(OperationResult {
                operation: operation_index,
                op: op_name,
                ok,
                error,
                emitted_events,
                notifications,
                sounds,
                status,
            });
        }
        frame_results.push(FrameResult {
            frame: frame_index,
            operations,
        });
    }

    // Treat scenario completion as a pause so the output state reflects the
    // same release behavior that a stopping engine guarantees.
    let cleanup_frame = frame_count.saturating_sub(1);
    let cleanup_operation = frame_results
        .last()
        .map_or(0, |frame| frame.operations.len());
    let cleanup_start = event_count(&state)?;
    let cleanup = engine.pause();
    let cleanup_events = events_since(&state, cleanup_start)?;
    let cleanup_input: Vec<_> = cleanup_events
        .iter()
        .map(|event| event.event.clone())
        .collect();
    let cleanup_status = engine.status();
    for event in cleanup_events {
        all_events.push(ObservedEvent {
            frame: cleanup_frame,
            operation: cleanup_operation,
            at_ms: event.at_ms,
            event: event.event,
        });
    }
    if let Some(frame) = frame_results.last_mut() {
        frame.operations.push(OperationResult {
            operation: cleanup_operation,
            op: "cleanup_pause".into(),
            ok: cleanup.is_ok(),
            error: cleanup.err(),
            emitted_events: cleanup_input,
            notifications: Vec::new(),
            sounds: Vec::new(),
            status: cleanup_status,
        });
    }
    let final_status = engine.status();
    // Engine::drop is a final backstop; include any release events it emits.
    let before_drop = event_count(&state)?;
    drop(engine);
    let trailing = events_since(&state, before_drop)?;
    for event in trailing {
        all_events.push(ObservedEvent {
            frame: cleanup_frame,
            operation: frame_results
                .last()
                .map_or(0, |frame| frame.operations.len()),
            at_ms: event.at_ms,
            event: event.event,
        });
    }

    let scripted = state.lock().map_err(|_| "simulation state lock poisoned")?;
    let notifications = scripted
        .notifications
        .iter()
        .map(|(message, ttl_ms)| SimulatedNotification {
            message: message.clone(),
            ttl_ms: *ttl_ms,
        })
        .collect();
    let sounds = scripted.sounds.clone();
    drop(scripted);
    let result = SimulationResult {
        schema_version: 1,
        frames: frame_results,
        events: all_events,
        rule_effects,
        notifications,
        sounds,
        final_status,
    };
    if serde_json::to_vec(&result)
        .map_err(|error| error.to_string())?
        .len()
        > MAX_RESULT_BYTES
    {
        return Err("simulation result exceeds the 8 MiB limit".into());
    }
    Ok(result)
}

pub fn validate_scenario(scenario: &SimulationScenario) -> Result<()> {
    if scenario.schema_version != 1 {
        return Err("simulation schema_version must be 1".into());
    }
    if scenario.frames.is_empty() || scenario.frames.len() > MAX_FRAMES {
        return Err("simulation requires 1..=10000 frames".into());
    }
    if scenario.frames[0].at_ms != 0 {
        return Err("the first simulation frame must start at 0 ms".into());
    }
    let count: usize = scenario
        .frames
        .iter()
        .map(|frame| frame.operations.len())
        .sum();
    if count > MAX_OPERATIONS {
        return Err("simulation has too many operations".into());
    }
    let mut total_wait_ms = 0u64;
    let mut total_observation_bytes = 0usize;
    let mut total_observation_items = 0usize;
    let mut prior_time = None;
    for frame in &scenario.frames {
        if frame.at_ms > 86_400_000 || prior_time.is_some_and(|previous| frame.at_ms <= previous) {
            return Err("frame times must increase strictly within 24 hours".into());
        }
        prior_time = Some(frame.at_ms);
        let observation = &frame.observation;
        total_observation_items = total_observation_items
            .saturating_add(observation.keys_down.len())
            .saturating_add(observation.pixels.len())
            .saturating_add(observation.ocr.len());
        if total_observation_items > MAX_TRACE_EVENTS {
            return Err("scenario observation count exceeds the limit".into());
        }
        total_observation_bytes = total_observation_bytes
            .saturating_add(observation.clipboard.len())
            .saturating_add(
                observation
                    .ocr
                    .iter()
                    .fold(0usize, |total, item| total.saturating_add(item.text.len())),
            );
        if total_observation_bytes > MAX_EFFECT_BYTES {
            return Err("scenario clipboard observations exceed the size limit".into());
        }
        if observation.keys_down.len() > MAX_KEYS
            || observation.pixels.len() > MAX_PIXELS
            || observation.ocr.len() > MAX_OCR_BOXES
        {
            return Err("observation exceeds supported limits".into());
        }
        if observation
            .keys_down
            .iter()
            .any(|key| !(1..=0xff).contains(key))
        {
            return Err("observed keys must be in the 1..=255 range".into());
        }
        if observation
            .pixels
            .iter()
            .any(|p| !valid_point(p.point) || p.color > 0x00ff_ffff)
        {
            return Err("pixel observations require normalized points and RGB colors".into());
        }
        if observation.ocr.iter().any(|item| {
            item.text.is_empty()
                || item.text.len() > MAX_OCR_TEXT_BYTES
                || !valid_region(item.region)
        }) {
            return Err("OCR observations require bounded text and normalized regions".into());
        }
        for operation in &frame.operations {
            match operation {
                SimulationOperation::Execute { actions } => {
                    crate::runtime::validate_actions(actions, 0)?;
                    total_wait_ms = total_wait_ms.saturating_add(
                        actions
                            .iter()
                            .filter_map(|action| match action {
                                Action::Wait { ms } => Some(*ms),
                                Action::ClickBurst {
                                    count, interval_ms, ..
                                } => Some((*count as u64).saturating_mul(*interval_ms)),
                                _ => None,
                            })
                            .sum::<u64>(),
                    );
                }
                SimulationOperation::Watchdog {
                    timeout_ms: Some(ms),
                } if *ms > 86_400_000 => {
                    return Err("watchdog timeout exceeds 24 hours".into());
                }
                SimulationOperation::AssertCondition { condition, .. } => {
                    crate::runtime::validate_condition(condition, 0)?;
                }
                _ => {}
            }
        }
    }
    if scenario
        .frames
        .last()
        .is_some_and(|frame| frame.at_ms.saturating_add(total_wait_ms) > 86_400_000)
    {
        return Err("scenario time including waits exceeds 24 hours".into());
    }
    Ok(())
}

fn run_operation(
    engine: &mut Engine<SimulationBackend>,
    operation: &SimulationOperation,
) -> Result<()> {
    match operation {
        SimulationOperation::Arm => engine.arm(),
        SimulationOperation::Tick => engine.tick(),
        SimulationOperation::Execute { actions } => engine.execute(actions),
        SimulationOperation::Pause => engine.pause(),
        SimulationOperation::Abort => engine.abort(),
        SimulationOperation::Heartbeat => {
            engine.heartbeat();
            Ok(())
        }
        SimulationOperation::Watchdog { timeout_ms } => {
            engine.set_watchdog(timeout_ms.map(std::time::Duration::from_millis));
            Ok(())
        }
        SimulationOperation::AssertCondition {
            condition,
            expected,
        } => {
            let actual = engine.matches(condition)?;
            if actual == *expected {
                Ok(())
            } else {
                Err(format!(
                    "condition assertion failed: expected {expected}, got {actual}"
                ))
            }
        }
        SimulationOperation::AssertState { expected } => {
            let status = engine.status();
            for (field, wanted) in [
                ("armed", expected.armed),
                ("paused", expected.paused),
                ("abort_latched", expected.abort_latched),
                ("dormant", expected.dormant),
                ("hotkeys_enabled", expected.hotkeys_enabled),
            ] {
                if let Some(wanted) = wanted {
                    let actual = status[field]
                        .as_bool()
                        .ok_or_else(|| format!("status field {field} is not boolean"))?;
                    if actual != wanted {
                        return Err(format!(
                            "state assertion failed for {field}: expected {wanted}, got {actual}"
                        ));
                    }
                }
            }
            for (field, wanted) in [
                ("held_keys", expected.held_keys),
                ("held_buttons", expected.held_buttons),
            ] {
                if let Some(wanted) = wanted {
                    let actual = status[field]
                        .as_u64()
                        .ok_or_else(|| format!("status field {field} is not an integer"))?
                        as usize;
                    if actual != wanted {
                        return Err(format!(
                            "state assertion failed for {field}: expected {wanted}, got {actual}"
                        ));
                    }
                }
            }
            Ok(())
        }
    }
}

impl SimulationOperation {
    fn name(&self) -> &'static str {
        match self {
            Self::Arm => "arm",
            Self::Tick => "tick",
            Self::Execute { .. } => "execute",
            Self::Pause => "pause",
            Self::Abort => "abort",
            Self::Heartbeat => "heartbeat",
            Self::Watchdog { .. } => "watchdog",
            Self::AssertCondition { .. } => "assert_condition",
            Self::AssertState { .. } => "assert_state",
        }
    }
}

fn valid_point(point: Point) -> bool {
    point.x.is_finite()
        && point.y.is_finite()
        && (0.0..=1.0).contains(&point.x)
        && (0.0..=1.0).contains(&point.y)
}
fn valid_region(region: Region) -> bool {
    region.x.is_finite()
        && region.y.is_finite()
        && region.width.is_finite()
        && region.height.is_finite()
        && region.x >= 0.0
        && region.y >= 0.0
        && region.width > 0.0
        && region.height > 0.0
        && region.x + region.width <= 1.0
        && region.y + region.height <= 1.0
}
fn regions_overlap(left: Region, right: Region) -> bool {
    left.x < right.x + right.width
        && right.x < left.x + left.width
        && left.y < right.y + right.height
        && right.y < left.y + left.height
}
fn event_count(state: &Arc<Mutex<ScriptedState>>) -> Result<usize> {
    Ok(state
        .lock()
        .map_err(|_| "simulation state lock poisoned")?
        .events
        .len())
}
fn effect_counts(state: &Arc<Mutex<ScriptedState>>) -> Result<(usize, usize)> {
    let state = state.lock().map_err(|_| "simulation state lock poisoned")?;
    Ok((state.notifications.len(), state.sounds.len()))
}
fn effects_since(
    state: &Arc<Mutex<ScriptedState>>,
    notification_start: usize,
    sound_start: usize,
) -> Result<(Vec<SimulatedNotification>, Vec<SoundCue>)> {
    let state = state.lock().map_err(|_| "simulation state lock poisoned")?;
    let notifications = state
        .notifications
        .get(notification_start..)
        .unwrap_or_default()
        .iter()
        .map(|(message, ttl_ms)| SimulatedNotification {
            message: message.clone(),
            ttl_ms: *ttl_ms,
        })
        .collect();
    let sounds = state.sounds.get(sound_start..).unwrap_or_default().to_vec();
    Ok((notifications, sounds))
}
fn events_since(state: &Arc<Mutex<ScriptedState>>, start: usize) -> Result<Vec<TimedInputEvent>> {
    Ok(state
        .lock()
        .map_err(|_| "simulation state lock poisoned")?
        .events
        .get(start..)
        .unwrap_or_default()
        .to_vec())
}
fn set_virtual_time(state: &Arc<Mutex<ScriptedState>>, at_ms: u64) -> Result<()> {
    let mut scripted = state.lock().map_err(|_| "simulation state lock poisoned")?;
    scripted.virtual_time_ms = scripted.virtual_time_ms.max(at_ms);
    let now = scripted.virtual_time_ms;
    apply_timeline(&mut scripted, now);
    Ok(())
}
fn apply_timeline(state: &mut ScriptedState, at_ms: u64) {
    if let Some((_, observation)) = state.timeline.iter().rev().find(|(time, _)| *time <= at_ms) {
        state.observation = Some(observation.clone());
    }
}

struct SimulationBackend {
    state: Arc<Mutex<ScriptedState>>,
}
impl SimulationBackend {
    fn observation(&self) -> Result<Observation> {
        self.state
            .lock()
            .map_err(|_| "simulation state lock poisoned")?
            .observation
            .clone()
            .ok_or_else(|| "simulation observation is not set".into())
    }
}
impl Backend for SimulationBackend {
    fn virtual_time_ms(&self) -> Option<u64> {
        self.state.lock().ok().map(|state| state.virtual_time_ms)
    }
    fn sleep_ms(&mut self, ms: u64) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "simulation state lock poisoned")?;
        let next = state.virtual_time_ms.saturating_add(ms);
        if next > 86_400_000 {
            return Err("simulation virtual time exceeded 24 hours".into());
        }
        state.virtual_time_ms = next;
        let now = state.virtual_time_ms;
        apply_timeline(&mut state, now);
        Ok(())
    }
    fn target_present(&mut self) -> Result<bool> {
        Ok(self.observation()?.target_present)
    }
    fn focused(&mut self) -> Result<bool> {
        Ok(self.observation()?.focused)
    }
    fn key_down(&mut self, key: u16) -> Result<bool> {
        let keys = self.observation()?.keys_down;
        Ok(keys.contains(&key)
            || match key {
                16 => keys.contains(&160) || keys.contains(&161),
                17 => keys.contains(&162) || keys.contains(&163),
                18 => keys.contains(&164) || keys.contains(&165),
                _ => false,
            })
    }
    fn cursor(&mut self) -> Result<Point> {
        Ok(self.observation()?.cursor)
    }
    fn pixel(&mut self, point: Point) -> Result<u32> {
        self.observation()?
            .pixels
            .into_iter()
            .find(|pixel| pixel.point.x == point.x && pixel.point.y == point.y)
            .map(|pixel| pixel.color)
            .ok_or_else(|| "pixel observation missing for requested point".into())
    }
    fn clipboard(&mut self) -> Result<String> {
        Ok(self.observation()?.clipboard)
    }
    fn emit(&mut self, event: &InputEvent) -> Result<()> {
        let encoded_len = serde_json::to_vec(event)
            .map_err(|error| error.to_string())?
            .len();
        let mut state = self
            .state
            .lock()
            .map_err(|_| "simulation state lock poisoned")?;
        if state.events.len() >= MAX_TRACE_EVENTS
            || state.trace_bytes.saturating_add(encoded_len) > MAX_TRACE_BYTES
        {
            return Err("simulation input trace limit reached".into());
        }
        state.trace_bytes += encoded_len;
        let at_ms = state.virtual_time_ms;
        state.events.push(TimedInputEvent {
            at_ms,
            event: event.clone(),
        });
        Ok(())
    }
    fn screenshot(&mut self, _region: Option<Region>) -> Result<Capture> {
        Err("screenshot observations are not supported by this simulator".into())
    }
    fn ocr(&mut self, region: Option<Region>) -> Result<Vec<TextBox>> {
        let boxes = self.observation()?.ocr;
        Ok(match region {
            Some(region) => boxes
                .into_iter()
                .filter(|item| regions_overlap(item.region, region))
                .collect(),
            None => boxes,
        })
    }
    fn notify(&mut self, message: &str, ttl_ms: u64) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "simulation state lock poisoned")?;
        let bytes = message.len();
        if state.notifications.len() >= MAX_EFFECTS
            || state.effect_bytes.saturating_add(bytes) > MAX_EFFECT_BYTES
        {
            return Err("simulation effect limit reached".into());
        }
        state.effect_bytes += bytes;
        state.notifications.push((message.to_string(), ttl_ms));
        Ok(())
    }
    fn sound(&mut self, cue: SoundCue) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "simulation state lock poisoned")?;
        if state.sounds.len() >= MAX_EFFECTS {
            return Err("simulation effect limit reached".into());
        }
        state.sounds.push(cue);
        Ok(())
    }
}
