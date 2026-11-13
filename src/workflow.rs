use crate::{
    Backend, Result,
    model::{Action, Condition, Step, Workflow},
    runtime::{self, Engine},
};
use serde_json::json;

const MAX_DEPTH: usize = 16;
const MAX_ACTIONS: u64 = 100_000;
const MAX_WAIT_MS: u64 = 3_600_000;

pub fn validate(workflow: &Workflow) -> Result<()> {
    if workflow.schema_version != 1 {
        return Err("unsupported workflow schema".into());
    }
    let mut total = 0u64;
    validate_steps(&workflow.steps, 0, &mut total)?;
    if count_actions(&workflow.steps)? > MAX_ACTIONS {
        return Err("expanded action count exceeds 100000".into());
    }
    if count_waits(&workflow.steps)? > MAX_WAIT_MS {
        return Err("expanded workflow wait exceeds one hour".into());
    }
    Ok(())
}

fn validate_steps(steps: &[Step], depth: usize, total: &mut u64) -> Result<()> {
    if depth > MAX_DEPTH {
        return Err("workflow nesting exceeds 16 levels".into());
    }
    for step in steps {
        match step {
            Step::Actions { actions } => {
                for action in actions {
                    validate_action(action)?;
                    *total = total
                        .checked_add(action_cost(action))
                        .ok_or("action count overflow")?;
                }
            }
            Step::WaitFor {
                condition,
                timeout_ms,
                poll_ms,
            } => {
                if *timeout_ms == 0
                    || *timeout_ms > MAX_WAIT_MS
                    || *poll_ms == 0
                    || *poll_ms > *timeout_ms
                {
                    return Err("invalid wait_for timing".into());
                }
                validate_condition(condition, 0)?;
            }
            Step::Assert { condition } => validate_condition(condition, 0)?,
            Step::Repeat { count, steps } => {
                if *count == 0 || *count > 100_000 {
                    return Err("repeat count is out of bounds".into());
                }
                let mut nested_total = 0;
                validate_steps(steps, depth + 1, &mut nested_total)?;
                let nested = count_actions(steps)?;
                *total = total
                    .checked_add(nested.saturating_mul(*count as u64))
                    .ok_or("action count overflow")?;
            }
        }
        if *total > MAX_ACTIONS {
            return Err("expanded action count exceeds 100000".into());
        }
    }
    Ok(())
}

fn count_actions(steps: &[Step]) -> Result<u64> {
    let mut n = 0u64;
    for s in steps {
        n = n
            .checked_add(match s {
                Step::Actions { actions } => actions.iter().map(action_cost).sum(),
                Step::Repeat { count, steps } => {
                    count_actions(steps)?.saturating_mul(*count as u64)
                }
                _ => 1,
            })
            .ok_or("action count overflow")?;
    }
    Ok(n)
}
fn action_cost(action: &Action) -> u64 {
    match action {
        Action::ClickBurst {
            count,
            lock_position,
            ..
        } => u64::from(*count) * if *lock_position { 3 } else { 2 },
        _ => 1,
    }
}
fn count_waits(steps: &[Step]) -> Result<u64> {
    let mut total = 0u64;
    for step in steps {
        let wait = match step {
            Step::Actions { actions } => {
                let mut duration = 0u64;
                for action in actions {
                    let value = match action {
                        Action::Wait { ms } => *ms,
                        Action::RandomWait { max_ms, .. } => *max_ms,
                        Action::ClickBurst {
                            count, interval_ms, ..
                        } => u64::from(*count)
                            .checked_mul(*interval_ms)
                            .ok_or("wait budget overflow")?,
                        _ => 0,
                    };
                    duration = duration.checked_add(value).ok_or("wait budget overflow")?;
                }
                duration
            }
            Step::WaitFor { timeout_ms, .. } => *timeout_ms,
            Step::Repeat { count, steps } => count_waits(steps)?
                .checked_mul(u64::from(*count))
                .ok_or("wait budget overflow")?,
            _ => 0,
        };
        total = total.checked_add(wait).ok_or("wait budget overflow")?;
    }
    Ok(total)
}

fn validate_action(action: &Action) -> Result<()> {
    runtime::validate_action(action)?;
    match action {
        Action::Tap { key } | Action::Key { key, .. } if *key == 0 || *key > 255 => {
            Err("key is out of bounds".into())
        }
        Action::Move { x, y }
            if !x.is_finite() || !y.is_finite() || *x < 0.0 || *x > 1.0 || *y < 0.0 || *y > 1.0 =>
        {
            Err("move coordinates must be normalized values in 0..=1".into())
        }
        Action::Click { button } | Action::Button { button, .. } if *button == 0 || *button > 8 => {
            Err("button is out of bounds".into())
        }
        Action::Wait { ms } if *ms == 0 || *ms > MAX_WAIT_MS => {
            Err("wait duration is out of bounds".into())
        }
        Action::Notify { message, ttl_ms } if message.len() > 4096 || *ttl_ms > MAX_WAIT_MS => {
            Err("notification is out of bounds".into())
        }
        Action::Text { text } if text.len() > 4096 => Err("text is too long".into()),
        _ => Ok(()),
    }
}

fn validate_condition(c: &Condition, depth: usize) -> Result<()> {
    if depth > MAX_DEPTH {
        return Err("condition nesting exceeds 16 levels".into());
    }
    runtime::validate_condition(c, depth)?;
    match c {
        Condition::Pixel { point, .. } if !point.x.is_finite() || !point.y.is_finite() => {
            Err("pixel coordinates must be finite".into())
        }
        Condition::ClipboardContains { text } if text.len() > 4096 => {
            Err("condition text is too long".into())
        }
        Condition::All { conditions } | Condition::Any { conditions } => {
            for c in conditions {
                validate_condition(c, depth + 1)?;
            }
            Ok(())
        }
        Condition::Not { condition } => validate_condition(condition, depth + 1),
        _ => Ok(()),
    }
}

pub fn run<B: Backend>(engine: &mut Engine<B>, workflow: &Workflow) -> Result<()> {
    validate(workflow)?;
    engine.safety_check()?;
    run_steps(engine, &workflow.steps)
}

fn run_steps<B: Backend>(engine: &mut Engine<B>, steps: &[Step]) -> Result<()> {
    for step in steps {
        engine.heartbeat();
        match step {
            Step::Actions { actions } => engine.execute(actions)?,
            Step::Assert { condition } => {
                if !engine.matches(condition)? {
                    return Err("workflow assertion failed".into());
                }
            }
            Step::Repeat { count, steps } => {
                for _ in 0..*count {
                    run_steps(engine, steps)?;
                }
            }
            Step::WaitFor {
                condition,
                timeout_ms,
                poll_ms,
            } => {
                let start = engine.elapsed_ms();
                loop {
                    engine.heartbeat();
                    if engine.matches(condition)? {
                        break;
                    }
                    if engine.elapsed_ms().saturating_sub(start) >= *timeout_ms {
                        return Err("wait_for timed out".into());
                    }
                    engine.wait_ms((*poll_ms).min(50).min(
                        timeout_ms.saturating_sub(engine.elapsed_ms().saturating_sub(start)),
                    ))?;
                }
            }
        }
    }
    Ok(())
}

pub fn dry_run(workflow: &Workflow) -> Result<serde_json::Value> {
    validate(workflow)?;
    Ok(
        json!({"schema_version": workflow.schema_version, "steps": workflow.steps, "effects": "none"}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_bad_schema_and_coordinates() {
        assert!(
            validate(&Workflow {
                schema_version: 2,
                steps: vec![]
            })
            .is_err()
        );
        assert!(
            validate(&Workflow {
                schema_version: 1,
                steps: vec![Step::Actions {
                    actions: vec![Action::Move {
                        x: f64::NAN,
                        y: 0.0
                    }]
                }]
            })
            .is_err()
        );
    }
    #[test]
    fn dry_run_is_serializable() {
        let w = Workflow {
            schema_version: 1,
            steps: vec![Step::Actions {
                actions: vec![Action::Wait { ms: 1 }],
            }],
        };
        assert_eq!(dry_run(&w).unwrap()["effects"], "none");
    }
    #[test]
    fn burst_work_and_total_wait_are_bounded() {
        let burst = Action::ClickBurst {
            button: 1,
            count: 1000,
            interval_ms: 5000,
            lock_position: true,
        };
        assert!(
            validate(&Workflow {
                schema_version: 1,
                steps: vec![Step::Actions {
                    actions: vec![burst]
                }]
            })
            .is_err()
        );
        let burst = Action::ClickBurst {
            button: 1,
            count: 1000,
            interval_ms: 1,
            lock_position: true,
        };
        assert!(
            validate(&Workflow {
                schema_version: 1,
                steps: vec![Step::Repeat {
                    count: 100,
                    steps: vec![Step::Actions {
                        actions: vec![burst]
                    }]
                }]
            })
            .is_err()
        );
    }

    #[test]
    fn random_wait_counts_maximum_toward_budget() {
        let workflow = Workflow {
            schema_version: 1,
            steps: vec![Step::Repeat {
                count: 3601,
                steps: vec![Step::Actions {
                    actions: vec![Action::RandomWait {
                        min_ms: 0,
                        max_ms: 1000,
                    }],
                }],
            }],
        };
        assert!(validate(&workflow).is_err());
    }

    #[test]
    fn ocr_checkpoints_require_nonempty_text_and_valid_region() {
        let valid = Condition::OcrContains {
            text: "ready".into(),
            region: Some(crate::model::Region {
                x: 0.1,
                y: 0.2,
                width: 0.5,
                height: 0.4,
            }),
            case_sensitive: false,
        };
        assert!(
            validate(&Workflow {
                schema_version: 1,
                steps: vec![Step::WaitFor {
                    condition: valid.clone(),
                    timeout_ms: 100,
                    poll_ms: 10
                }],
            })
            .is_ok()
        );
        for condition in [
            Condition::OcrContains {
                text: "  ".into(),
                region: None,
                case_sensitive: false,
            },
            Condition::OcrContains {
                text: "ready".into(),
                region: Some(crate::model::Region {
                    x: 0.9,
                    y: 0.0,
                    width: 0.2,
                    height: 0.2,
                }),
                case_sensitive: false,
            },
        ] {
            assert!(
                validate(&Workflow {
                    schema_version: 1,
                    steps: vec![Step::Assert { condition }],
                })
                .is_err()
            );
        }
    }
}
