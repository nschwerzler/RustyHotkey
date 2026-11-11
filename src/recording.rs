use crate::{
    Result,
    model::{Action, Controls, Point, RecordedEvent, Recording},
    runtime,
};
use serde_json::to_value;
use std::collections::HashSet;

const MOUSE_KEYS: &[(u16, u8)] = &[(1, 1), (2, 2), (4, 3), (5, 4), (6, 5)];

pub struct Recorder {
    controls: Controls,
    include_text: bool,
    held: HashSet<u16>,
    buttons: HashSet<u8>,
    last_cursor: Option<Point>,
    last_ms: Option<u64>,
    events: Vec<RecordedEvent>,
}

impl Recorder {
    pub fn new(controls: &Controls, include_text: bool) -> Self {
        Self {
            controls: controls.clone(),
            include_text,
            held: HashSet::new(),
            buttons: HashSet::new(),
            last_cursor: None,
            last_ms: None,
            events: Vec::new(),
        }
    }
    pub fn sample(&mut self, at_ms: u64, keys: &[u16], cursor: Point) -> Result<()> {
        if !cursor.x.is_finite()
            || !cursor.y.is_finite()
            || !(0.0..=1.0).contains(&cursor.x)
            || !(0.0..=1.0).contains(&cursor.y)
        {
            return Err("cursor coordinates must be normalized values in 0..=1".into());
        }
        if let Some(last) = self.last_ms {
            if at_ms < last {
                return Err("recording timestamps must be monotonic".into());
            }
        }
        self.last_ms = Some(at_ms);
        if self
            .last_cursor
            .map_or(true, |p| p.x != cursor.x || p.y != cursor.y)
        {
            self.events.push(RecordedEvent {
                at_ms,
                action: Action::Move {
                    x: cursor.x,
                    y: cursor.y,
                },
            });
            self.last_cursor = Some(cursor);
        }
        let current: HashSet<u16> = keys.iter().copied().collect();
        for &key in current.difference(&self.held) {
            if self.is_reserved(key) {
                continue;
            }
            if let Some(&(_, button)) = MOUSE_KEYS.iter().find(|&&(k, _)| k == key) {
                self.buttons.insert(button);
                self.events.push(RecordedEvent {
                    at_ms,
                    action: Action::Button { button, down: true },
                });
            } else if self.include_text || !is_printable(key) {
                self.events.push(RecordedEvent {
                    at_ms,
                    action: Action::Key { key, down: true },
                });
            }
        }
        for &key in self.held.difference(&current) {
            if let Some(&(_, button)) = MOUSE_KEYS.iter().find(|&&(k, _)| k == key) {
                self.buttons.remove(&button);
                self.events.push(RecordedEvent {
                    at_ms,
                    action: Action::Button {
                        button,
                        down: false,
                    },
                });
            } else if !self.is_reserved(key) && (self.include_text || !is_printable(key)) {
                self.events.push(RecordedEvent {
                    at_ms,
                    action: Action::Key { key, down: false },
                });
            }
        }
        self.held = current;
        Ok(())
    }
    pub fn finish(mut self) -> Recording {
        let at = self.last_ms.unwrap_or(0);
        let held: Vec<u16> = self.held.drain().collect();
        for key in held {
            if let Some(&(_, button)) = MOUSE_KEYS.iter().find(|&&(k, _)| k == key) {
                self.events.push(RecordedEvent {
                    at_ms: at,
                    action: Action::Button {
                        button,
                        down: false,
                    },
                });
            } else if !self.is_reserved(key) && (self.include_text || !is_printable(key)) {
                self.events.push(RecordedEvent {
                    at_ms: at,
                    action: Action::Key { key, down: false },
                });
            }
        }
        Recording {
            schema_version: 1,
            events: self.events,
        }
    }
    fn is_reserved(&self, key: u16) -> bool {
        [
            self.controls.abort,
            self.controls.reset,
            self.controls.pause,
            self.controls.resume,
        ]
        .contains(&key)
    }
}
fn is_printable(key: u16) -> bool {
    (0x20..=0x7e).contains(&key) || (0x30..=0x5a).contains(&key)
}
pub fn validate(recording: &Recording) -> Result<()> {
    if recording.schema_version != 1 {
        return Err("unsupported recording schema".into());
    }
    if recording.events.windows(2).any(|w| w[1].at_ms < w[0].at_ms) {
        return Err("recording timestamps must be monotonic".into());
    }
    Ok(())
}
pub fn plan(recording: &Recording, speed: f64) -> Result<Vec<Action>> {
    validate(recording)?;
    if !speed.is_finite() || !(0.1..=10.0).contains(&speed) {
        return Err("speed must be between 0.1 and 10".into());
    }
    let mut out = Vec::new();
    let mut previous = 0;
    let mut keys = HashSet::new();
    let mut buttons = HashSet::new();
    for event in &recording.events {
        runtime::validate_action(&event.action)?;
        let delay = ((event.at_ms - previous) as f64 / speed).round() as u64;
        if delay > 0 {
            out.push(Action::Wait { ms: delay });
        }
        match &event.action {
            Action::Key { key, down } => {
                if *down {
                    if !keys.insert(*key) {
                        return Err("replay contains a duplicate key press".into());
                    }
                } else if !keys.remove(key) {
                    return Err("replay releases a key that is not held".into());
                }
            }
            Action::Button { button, down } => {
                if *down {
                    if !buttons.insert(*button) {
                        return Err("replay contains a duplicate button press".into());
                    }
                } else if !buttons.remove(button) {
                    return Err("replay releases a button that is not held".into());
                }
            }
            _ => {}
        }
        out.push(event.action.clone());
        previous = event.at_ms;
    }
    if !keys.is_empty() || !buttons.is_empty() {
        return Err("replay contains dangling held inputs".into());
    }
    Ok(out)
}
pub fn describe(recording: &Recording) -> Result<serde_json::Value> {
    validate(recording)?;
    Ok(to_value(recording).map_err(|e| e.to_string())?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn records_motion_and_cleanup() {
        let c = Controls {
            abort: 99,
            reset: 98,
            pause: 97,
            resume: 96,
        };
        let mut r = Recorder::new(&c, true);
        r.sample(0, &[10], Point { x: 1.0, y: 0.2 }).unwrap();
        let x = r.finish();
        assert!(matches!(
            x.events.last().unwrap().action,
            Action::Key { down: false, .. }
        ));
    }
    #[test]
    fn redacts_printable_by_default() {
        let c = Controls {
            abort: 99,
            reset: 98,
            pause: 97,
            resume: 96,
        };
        let mut r = Recorder::new(&c, false);
        r.sample(0, &[65], Point::default()).unwrap();
        assert_eq!(r.finish().events.len(), 1);
    }
    #[test]
    fn plan_scales_time() {
        let x = Recording {
            schema_version: 1,
            events: vec![RecordedEvent {
                at_ms: 100,
                action: Action::Wait { ms: 1 },
            }],
        };
        assert!(matches!(plan(&x, 2.0).unwrap()[0], Action::Wait { ms: 50 }));
    }
}
