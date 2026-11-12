use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Region {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub window_class: Option<String>,
    pub window_title: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum InputEvent {
    Key { key: u16, down: bool },
    Move { x: f64, y: f64 },
    Button { button: u8, down: bool },
    Text { text: String },
    Wheel { delta: i32, horizontal: bool },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Tap {
        key: u16,
    },
    Key {
        key: u16,
        down: bool,
    },
    Move {
        x: f64,
        y: f64,
    },
    Click {
        button: u8,
    },
    ClickBurst {
        button: u8,
        count: u32,
        interval_ms: u64,
        lock_position: bool,
    },
    Button {
        button: u8,
        down: bool,
    },
    Text {
        text: String,
    },
    Scroll {
        delta: i32,
        #[serde(default)]
        horizontal: bool,
    },
    Wait {
        ms: u64,
    },
    Pause,
    SetHotkeys {
        enabled: bool,
    },
    ToggleHotkeys,
    Sound {
        cue: SoundCue,
    },
    Notify {
        message: String,
        ttl_ms: u64,
    },
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SoundCue {
    Default,
    Info,
    Warning,
    Error,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Condition {
    Always,
    Focused,
    Modifiers {
        #[serde(default)]
        shift: Option<bool>,
        #[serde(default)]
        control: Option<bool>,
        #[serde(default)]
        alt: Option<bool>,
        #[serde(default)]
        meta: Option<bool>,
    },
    KeyDown {
        key: u16,
    },
    Pixel {
        point: Point,
        color: u32,
        tolerance: u8,
    },
    PixelPattern {
        point: Point,
        pattern: String,
    },
    ClipboardContains {
        text: String,
    },
    All {
        conditions: Vec<Condition>,
    },
    Any {
        conditions: Vec<Condition>,
    },
    Not {
        condition: Box<Condition>,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub name: String,
    pub condition: Condition,
    pub actions: Vec<Action>,
    pub cooldown_ms: u64,
    #[serde(default)]
    pub delay_ms: u64,
    #[serde(default = "yes")]
    pub enabled: bool,
}
fn yes() -> bool {
    true
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Controls {
    pub abort: u16,
    pub reset: u16,
    pub pause: u16,
    pub resume: u16,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub schema_version: u32,
    pub target: Target,
    pub controls: Controls,
    pub tick_ms: u64,
    #[serde(default)]
    pub rules: Vec<Rule>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Step {
    Actions {
        actions: Vec<Action>,
    },
    WaitFor {
        condition: Condition,
        timeout_ms: u64,
        poll_ms: u64,
    },
    Assert {
        condition: Condition,
    },
    Repeat {
        count: u32,
        steps: Vec<Step>,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workflow {
    pub schema_version: u32,
    pub steps: Vec<Step>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedEvent {
    pub at_ms: u64,
    pub action: Action,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recording {
    pub schema_version: u32,
    pub events: Vec<RecordedEvent>,
}
#[derive(Debug)]
pub struct Capture {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TextBox {
    pub text: String,
    pub region: Region,
}
