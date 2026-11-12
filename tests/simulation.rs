use rustyhotkey::{Result, model::*, simulation::*};
use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

#[test]
fn delayed_rules_do_not_block_and_cancel_when_condition_changes() {
    let delayed = Rule {
        name: "delayed".into(),
        condition: Condition::KeyDown { key: 65 },
        actions: vec![Action::Tap { key: 66 }],
        cooldown_ms: 100,
        delay_ms: 100,
        enabled: true,
    };
    let immediate = Rule {
        name: "immediate".into(),
        condition: Condition::Always,
        actions: vec![Action::Tap { key: 67 }],
        cooldown_ms: 1000,
        delay_ms: 0,
        enabled: true,
    };
    let mut held = observation(true, true);
    held.keys_down = vec![65];
    let result = run(scenario(
        profile(vec![delayed, immediate]),
        vec![
            frame(
                0,
                held.clone(),
                vec![SimulationOperation::Arm, SimulationOperation::Tick],
            ),
            frame(50, observation(true, true), vec![SimulationOperation::Tick]),
            frame(100, held.clone(), vec![SimulationOperation::Tick]),
            frame(199, held.clone(), vec![SimulationOperation::Tick]),
            frame(200, held, vec![SimulationOperation::Tick]),
        ],
    ))
    .unwrap();
    assert_eq!(
        result.frames[0].operations[1].status["pending_rules"][0]["remaining_ms"],
        100
    );
    let down: Vec<_> = result
        .events
        .iter()
        .filter_map(|event| match event.event {
            InputEvent::Key { key, down: true } => Some((event.at_ms, key)),
            _ => None,
        })
        .collect();
    assert_eq!(down, vec![(0, 67), (200, 66)]);
}

#[test]
fn modifier_guards_and_scroll_are_simulated() {
    let mut sample = observation(true, true);
    sample.keys_down = vec![160];
    let result = run(scenario(
        profile(vec![]),
        vec![frame(
            0,
            sample,
            vec![
                SimulationOperation::Arm,
                SimulationOperation::AssertCondition {
                    condition: Condition::Modifiers {
                        shift: Some(true),
                        control: None,
                        alt: Some(false),
                        meta: None,
                    },
                    expected: true,
                },
                SimulationOperation::Execute {
                    actions: vec![Action::Scroll {
                        delta: -120,
                        horizontal: false,
                    }],
                },
            ],
        )],
    ))
    .unwrap();
    assert!(
        result.frames[0]
            .operations
            .iter()
            .all(|operation| operation.ok)
    );
    assert!(matches!(
        result.events[0].event,
        InputEvent::Wheel {
            delta: -120,
            horizontal: false
        }
    ));
}

fn observation(target_present: bool, focused: bool) -> Observation {
    Observation {
        target_present,
        focused,
        keys_down: vec![],
        pixels: vec![PixelObservation {
            point: Point { x: 0.5, y: 0.5 },
            color: 0x123456,
        }],
        clipboard: "ready".into(),
        ocr: vec![],
        cursor: Point { x: 0.5, y: 0.5 },
        screenshot: None,
    }
}

fn profile(rules: Vec<Rule>) -> Profile {
    Profile {
        schema_version: 1,
        target: Target {
            window_class: Some("FixtureWindow".into()),
            window_title: None,
        },
        controls: Controls {
            abort: 0x70,
            reset: 0x71,
            pause: 0x72,
            resume: 0x73,
        },
        tick_ms: 10,
        rules,
    }
}

fn scenario(profile: Profile, frames: Vec<SimulationFrame>) -> SimulationScenario {
    SimulationScenario {
        schema_version: 1,
        profile,
        frames,
    }
}

fn frame(
    at_ms: u64,
    observation: Observation,
    operations: Vec<SimulationOperation>,
) -> SimulationFrame {
    SimulationFrame {
        at_ms,
        observation,
        operations,
    }
}

fn run(input: SimulationScenario) -> Result<SimulationResult> {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let app_data = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let dir: PathBuf = app_data
        .join("RustyHotkey")
        .join("simulation-tests")
        .join(format!("{}-{id}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let result = run_scenario(input, &dir);
    let cleanup = fs::remove_dir_all(&dir).map_err(|e| e.to_string());
    match (result, cleanup) {
        (Ok(result), Ok(())) => Ok(result),
        (Err(error), _) | (_, Err(error)) => Err(error),
    }
}

fn emitted(result: &SimulationResult) -> Vec<serde_json::Value> {
    result
        .events
        .iter()
        .map(|event| serde_json::to_value(&event.event).unwrap())
        .collect()
}

#[test]
fn records_modifier_chords_and_releases() {
    let result = run(scenario(
        profile(vec![]),
        vec![frame(
            0,
            observation(true, true),
            vec![
                SimulationOperation::Arm,
                SimulationOperation::Execute {
                    actions: vec![
                        Action::Key {
                            key: 0x11,
                            down: true,
                        },
                        Action::Tap { key: b'A' as u16 },
                        Action::Key {
                            key: 0x11,
                            down: false,
                        },
                    ],
                },
                SimulationOperation::AssertCondition {
                    condition: Condition::Pixel {
                        point: Point { x: 0.5, y: 0.5 },
                        color: 0x123456,
                        tolerance: 0,
                    },
                    expected: true,
                },
                SimulationOperation::AssertCondition {
                    condition: Condition::ClipboardContains {
                        text: "ready".into(),
                    },
                    expected: true,
                },
            ],
        )],
    ))
    .unwrap();
    assert_eq!(
        emitted(&result),
        vec![
            json!({"type":"key","key":17,"down":true}),
            json!({"type":"key","key":65,"down":true}),
            json!({"type":"key","key":65,"down":false}),
            json!({"type":"key","key":17,"down":false}),
        ]
    );
    assert_eq!(result.final_status["held_keys"], 0);
}

#[test]
fn focus_loss_during_wait_releases_held_modifier_before_more_input() {
    let result = run(scenario(
        profile(vec![]),
        vec![
            frame(
                0,
                observation(true, true),
                vec![
                    SimulationOperation::Arm,
                    SimulationOperation::Execute {
                        actions: vec![
                            Action::Key {
                                key: 0x10,
                                down: true,
                            },
                            Action::Wait { ms: 20 },
                            Action::Tap { key: b'A' as u16 },
                        ],
                    },
                ],
            ),
            frame(
                10,
                observation(true, false),
                vec![SimulationOperation::AssertState {
                    expected: StateExpectation {
                        armed: Some(false),
                        paused: Some(true),
                        held_keys: Some(0),
                        ..Default::default()
                    },
                }],
            ),
        ],
    ))
    .unwrap();
    assert_eq!(
        emitted(&result),
        vec![
            json!({"type":"key","key":16,"down":true}),
            json!({"type":"key","key":16,"down":false}),
        ]
    );
    assert_eq!(result.frames[0].operations[1].ok, false);
    assert_eq!(
        result.frames[0].operations[1].error.as_deref(),
        Some("target lost focus")
    );
}

#[test]
fn target_absence_is_dormant_and_emits_no_input() {
    let result = run(scenario(
        profile(vec![]),
        vec![
            frame(
                0,
                observation(false, false),
                vec![
                    SimulationOperation::Tick,
                    SimulationOperation::AssertState {
                        expected: StateExpectation {
                            dormant: Some(true),
                            armed: Some(false),
                            ..Default::default()
                        },
                    },
                ],
            ),
            frame(
                25,
                observation(true, true),
                vec![
                    SimulationOperation::Tick,
                    SimulationOperation::AssertState {
                        expected: StateExpectation {
                            dormant: Some(false),
                            ..Default::default()
                        },
                    },
                ],
            ),
        ],
    ))
    .unwrap();
    assert!(result.events.is_empty());
    assert_eq!(result.frames[0].operations[1].ok, true);
    assert_eq!(result.frames[1].operations[1].ok, true);
}

#[test]
fn pause_releases_and_arm_resumes_safely_then_abort_latches() {
    let result = run(scenario(
        profile(vec![]),
        vec![frame(
            0,
            observation(true, true),
            vec![
                SimulationOperation::Arm,
                SimulationOperation::Execute {
                    actions: vec![Action::Key {
                        key: 0x11,
                        down: true,
                    }],
                },
                SimulationOperation::Pause,
                SimulationOperation::AssertState {
                    expected: StateExpectation {
                        paused: Some(true),
                        held_keys: Some(0),
                        ..Default::default()
                    },
                },
                SimulationOperation::Arm,
                SimulationOperation::Execute {
                    actions: vec![Action::Tap { key: b'B' as u16 }],
                },
                SimulationOperation::Execute {
                    actions: vec![Action::Key {
                        key: 0x10,
                        down: true,
                    }],
                },
                SimulationOperation::Abort,
                SimulationOperation::AssertState {
                    expected: StateExpectation {
                        abort_latched: Some(true),
                        held_keys: Some(0),
                        armed: Some(false),
                        ..Default::default()
                    },
                },
                SimulationOperation::Execute {
                    actions: vec![Action::Tap { key: b'C' as u16 }],
                },
            ],
        )],
    ))
    .unwrap();
    assert_eq!(
        emitted(&result),
        vec![
            json!({"type":"key","key":17,"down":true}),
            json!({"type":"key","key":17,"down":false}),
            json!({"type":"key","key":66,"down":true}),
            json!({"type":"key","key":66,"down":false}),
            json!({"type":"key","key":16,"down":true}),
            json!({"type":"key","key":16,"down":false}),
        ]
    );
    assert_eq!(result.frames[0].operations[9].ok, false);
    assert_eq!(
        result.frames[0].operations[9].error.as_deref(),
        Some("engine is not ready")
    );
}

#[test]
fn cooldown_uses_virtual_scenario_time_and_reports_tick_effects() {
    let rule = Rule {
        name: "sample action".into(),
        condition: Condition::Always,
        actions: vec![Action::Tap { key: b'A' as u16 }],
        cooldown_ms: 100,
        delay_ms: 0,
        enabled: true,
    };
    let result = run(scenario(
        profile(vec![rule]),
        vec![
            frame(
                0,
                observation(true, true),
                vec![SimulationOperation::Arm, SimulationOperation::Tick],
            ),
            frame(50, observation(true, true), vec![SimulationOperation::Tick]),
            frame(
                100,
                observation(true, true),
                vec![SimulationOperation::Tick],
            ),
        ],
    ))
    .unwrap();
    assert_eq!(result.events.len(), 4);
    assert_eq!(
        result
            .rule_effects
            .iter()
            .map(|effect| effect.events.len())
            .collect::<Vec<_>>(),
        vec![2, 0, 2]
    );
}

#[test]
fn click_burst_advances_virtual_time_and_pixel_patterns_use_scripted_samples() {
    let result = run(scenario(
        profile(vec![]),
        vec![frame(
            0,
            observation(true, true),
            vec![
                SimulationOperation::Arm,
                SimulationOperation::AssertCondition {
                    condition: Condition::PixelPattern {
                        point: Point { x: 0.5, y: 0.5 },
                        pattern: "12?456".into(),
                    },
                    expected: true,
                },
                SimulationOperation::Execute {
                    actions: vec![Action::ClickBurst {
                        button: 1,
                        count: 2,
                        interval_ms: 5,
                        lock_position: true,
                    }],
                },
            ],
        )],
    ))
    .unwrap();
    assert_eq!(
        result
            .events
            .iter()
            .map(|event| event.at_ms)
            .collect::<Vec<_>>(),
        vec![5, 5, 5, 10, 10, 10]
    );
    assert_eq!(
        emitted(&result),
        vec![
            json!({"type":"move","x":0.5,"y":0.5}),
            json!({"type":"button","button":1,"down":true}),
            json!({"type":"button","button":1,"down":false}),
            json!({"type":"move","x":0.5,"y":0.5}),
            json!({"type":"button","button":1,"down":true}),
            json!({"type":"button","button":1,"down":false}),
        ]
    );
}

#[test]
fn tick_effects_include_simulated_notification_and_sound() {
    let rule = Rule {
        name: "feedback".into(),
        condition: Condition::Always,
        actions: vec![
            Action::Notify {
                message: "condition matched".into(),
                ttl_ms: 500,
            },
            Action::Sound {
                cue: SoundCue::Info,
            },
        ],
        cooldown_ms: 0,
        delay_ms: 0,
        enabled: true,
    };
    let result = run(scenario(
        profile(vec![rule]),
        vec![frame(
            0,
            observation(true, true),
            vec![SimulationOperation::Arm, SimulationOperation::Tick],
        )],
    ))
    .unwrap();
    assert_eq!(result.notifications[0].message, "condition matched");
    assert_eq!(
        serde_json::to_value(&result.sounds).unwrap(),
        json!(["info"])
    );
    assert_eq!(
        result.rule_effects[0].notifications[0].message,
        "condition matched"
    );
    assert_eq!(
        serde_json::to_value(&result.rule_effects[0].sounds).unwrap(),
        json!(["info"])
    );
}

#[test]
fn ocr_conditions_match_scripted_text_and_filter_by_region() {
    let mut sample = observation(true, true);
    sample.ocr = vec![
        TextBox {
            text: "Ready to continue".into(),
            region: Region {
                x: 0.1,
                y: 0.2,
                width: 0.2,
                height: 0.1,
            },
        },
        TextBox {
            text: "Other area".into(),
            region: Region {
                x: 0.7,
                y: 0.7,
                width: 0.2,
                height: 0.1,
            },
        },
    ];
    let result = run(scenario(
        profile(vec![]),
        vec![frame(
            0,
            sample,
            vec![
                SimulationOperation::AssertCondition {
                    condition: Condition::OcrContains {
                        text: "READY TO".into(),
                        region: None,
                        case_sensitive: false,
                    },
                    expected: true,
                },
                SimulationOperation::AssertCondition {
                    condition: Condition::OcrContains {
                        text: "Other".into(),
                        region: Some(Region {
                            x: 0.0,
                            y: 0.0,
                            width: 0.5,
                            height: 0.5,
                        }),
                        case_sensitive: false,
                    },
                    expected: false,
                },
                SimulationOperation::AssertCondition {
                    condition: Condition::OcrContains {
                        text: "ready".into(),
                        region: None,
                        case_sensitive: true,
                    },
                    expected: false,
                },
            ],
        )],
    ))
    .unwrap();
    assert!(
        result.frames[0]
            .operations
            .iter()
            .all(|operation| operation.ok)
    );
    assert!(result.events.is_empty());
}

#[test]
fn simulation_rejects_unbounded_or_invalid_ocr_observations() {
    let mut sample = observation(true, true);
    sample.ocr.push(TextBox {
        text: "text".into(),
        region: Region {
            x: 0.9,
            y: 0.9,
            width: 0.2,
            height: 0.2,
        },
    });
    assert!(validate_scenario(&scenario(profile(vec![]), vec![frame(0, sample, vec![])])).is_err());

    let mut sample = observation(true, true);
    sample.ocr.push(TextBox {
        text: "x".repeat(4097),
        region: Region {
            x: 0.1,
            y: 0.1,
            width: 0.2,
            height: 0.2,
        },
    });
    assert!(validate_scenario(&scenario(profile(vec![]), vec![frame(0, sample, vec![])])).is_err());
}
