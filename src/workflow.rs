use crate::{
    Backend, Result,
    model::{Action, Condition, Step, Workflow},
    runtime::{self, Engine},
};
use serde_json::json;
use std::time::{Duration, Instant};

const MAX_DEPTH: usize = 16;
const MAX_ACTIONS: u64 = 100_000;
const MAX_WAIT_MS: u64 = 3_600_000;

pub fn validate(workflow: &Workflow) -> Result<()> {
    if workflow.schema_version != 1 {
        return Err("unsupported workflow schema".into());
    }
    let mut total = 0u64;
    validate_steps(&workflow.steps, 0, &mut total)
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
                    *total = total.checked_add(1).ok_or("action count overflow")?;
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
                Step::Actions { actions } => actions.len() as u64,
                Step::Repeat { count, steps } => {
                    count_actions(steps)?.saturating_mul(*count as u64)
                }
                _ => 0,
            })
            .ok_or("action count overflow")?;
    }
    Ok(n)
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
                let start = Instant::now();
                loop {
                    engine.heartbeat();
                    if engine.matches(condition)? {
                        break;
                    }
                    if start.elapsed() >= Duration::from_millis(*timeout_ms) {
                        return Err("wait_for timed out".into());
                    }
                    std::thread::sleep(Duration::from_millis((*poll_ms).min(50)));
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
}
