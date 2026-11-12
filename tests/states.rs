use rustyhotkey::{Backend, Result, model::*, runtime::Engine};
use std::{
    collections::HashSet,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

struct Fake {
    present: bool,
    focused: bool,
    keys: HashSet<u16>,
    events: Vec<InputEvent>,
}
impl Default for Fake {
    fn default() -> Self {
        Self {
            present: true,
            focused: true,
            keys: HashSet::new(),
            events: vec![],
        }
    }
}
impl Backend for Fake {
    fn target_present(&mut self) -> Result<bool> {
        Ok(self.present)
    }
    fn focused(&mut self) -> Result<bool> {
        Ok(self.focused)
    }
    fn key_down(&mut self, key: u16) -> Result<bool> {
        Ok(self.keys.contains(&key))
    }
    fn cursor(&mut self) -> Result<Point> {
        Ok(Point::default())
    }
    fn pixel(&mut self, _: Point) -> Result<u32> {
        Ok(0)
    }
    fn clipboard(&mut self) -> Result<String> {
        Ok(String::new())
    }
    fn emit(&mut self, event: &InputEvent) -> Result<()> {
        self.events.push(event.clone());
        Ok(())
    }
    fn screenshot(&mut self, _: Option<Region>) -> Result<Capture> {
        Err("not needed".into())
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
            window_class: Some("SyntheticWindow".into()),
            window_title: None,
        },
        controls: Controls {
            abort: 201,
            reset: 202,
            pause: 203,
            resume: 204,
        },
        tick_ms: 10,
        rules: vec![],
    }
}
fn dir() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "rhk-state-test-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn dormant_is_loaded_with_effective_hotkeys_disabled() {
    let path = dir();
    let mut engine = Engine::new(
        Fake {
            present: false,
            focused: false,
            ..Default::default()
        },
        profile(),
        path.clone(),
    )
    .unwrap();
    assert_eq!(engine.status()["dormant"], true);
    assert_eq!(engine.status()["running"], true);
    assert_eq!(engine.status()["hotkeys_enabled"], false);
    assert_eq!(engine.status()["requested_hotkeys"], true);
    engine.tick().unwrap();
    engine.set_hotkeys_enabled(false).unwrap();
    engine.backend.present = true;
    engine.backend.focused = true;
    engine.tick().unwrap();
    assert!(!engine.armed());
    assert_eq!(engine.status()["hotkeys_enabled"], false);
    drop(engine);
    fs::remove_dir_all(path).unwrap();
}
#[test]
fn disabled_keyboard_rules_do_not_stop_timer_rules() {
    let path = dir();
    let mut p = profile();
    p.rules = vec![
        Rule {
            name: "keyboard".into(),
            condition: Condition::Not {
                condition: Box::new(Condition::KeyDown { key: 65 }),
            },
            actions: vec![Action::Tap { key: 66 }],
            cooldown_ms: 0,
            delay_ms: 0,
            enabled: true,
        },
        Rule {
            name: "timer".into(),
            condition: Condition::Always,
            actions: vec![Action::Tap { key: 70 }],
            cooldown_ms: 0,
            delay_ms: 0,
            enabled: true,
        },
    ];
    let mut engine = Engine::new(Fake::default(), p, path.clone()).unwrap();
    engine.arm().unwrap();
    engine.set_hotkeys_enabled(false).unwrap();
    engine.tick().unwrap();
    assert!(engine.armed());
    assert!(
        !engine
            .backend
            .events
            .iter()
            .any(|e| matches!(e, InputEvent::Key { key: 66, .. }))
    );
    assert!(engine.backend.events.iter().any(|e| matches!(
        e,
        InputEvent::Key {
            key: 70,
            down: true
        }
    )));
    drop(engine);
    fs::remove_dir_all(path).unwrap();
}
#[test]
fn focus_loss_releases_owned_inputs() {
    let path = dir();
    let mut engine = Engine::new(Fake::default(), profile(), path.clone()).unwrap();
    engine.arm().unwrap();
    engine
        .execute(&[Action::Key {
            key: 70,
            down: true,
        }])
        .unwrap();
    engine.backend.focused = false;
    assert!(engine.tick().is_err());
    assert!(!engine.armed());
    assert!(matches!(
        engine.backend.events.last(),
        Some(InputEvent::Key {
            key: 70,
            down: false
        })
    ));
    drop(engine);
    fs::remove_dir_all(path).unwrap();
}
#[test]
fn pause_all_reaches_every_engine_and_does_not_latch() {
    let path = dir();
    let mut first = Engine::new(Fake::default(), profile(), path.clone()).unwrap();
    let mut second = Engine::new(Fake::default(), profile(), path.clone()).unwrap();
    first.arm().unwrap();
    second.arm().unwrap();
    fs::write(path.join("pause.signal"), b"new-pause").unwrap();
    first.poll_controls().unwrap();
    second.poll_controls().unwrap();
    assert!(!first.armed() && !second.armed());
    assert!(!first.latched() && !second.latched());
    first.arm().unwrap();
    first.poll_controls().unwrap();
    assert!(first.armed());
    drop(first);
    drop(second);
    fs::remove_dir_all(path).unwrap();
}
#[test]
fn script_hotkeys_toggle_is_consumed_and_emergency_stays_available() {
    let path = dir();
    let mut engine = Engine::new(Fake::default(), profile(), path.clone()).unwrap();
    engine.arm().unwrap();
    let control = path.join("control");
    fs::create_dir_all(&control).unwrap();
    let marker = control.join(format!("{}.hotkeys", std::process::id()));
    fs::write(&marker, b"{\"enabled\":false}").unwrap();
    engine.poll_controls().unwrap();
    assert!(!marker.exists());
    assert!(engine.armed());
    assert_eq!(engine.status()["hotkeys_enabled"], false);
    engine.set_hotkeys_enabled(true).unwrap();
    engine.poll_controls().unwrap();
    assert_eq!(engine.status()["hotkeys_enabled"], true);
    engine.set_hotkeys_enabled(false).unwrap();
    engine.backend.keys.insert(201);
    engine.poll_controls().unwrap();
    assert!(engine.latched());
    assert!(!engine.armed());
    drop(engine);
    fs::remove_dir_all(path).unwrap();
}
