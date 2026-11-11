//! Sandboxed, caller-selected Lua widgets for generic overlays.

use crate::overlay::OverlaySpec;
use mlua::{Function, HookTriggers, Lua, LuaOptions, LuaSerdeExt, StdLib, Value, VmState};
use serde_json::Value as JsonValue;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::SystemTime,
};

const INSTRUCTION_LIMIT: usize = 100_000;
const MAX_WIDGETS: usize = 64;
const MAX_SOURCE_BYTES: usize = 1 << 20;
const MAX_MEMORY_BYTES: usize = 8 << 20;

#[derive(Clone, Debug)]
pub struct WidgetDiagnostic {
    pub path: PathBuf,
    pub message: String,
}

#[derive(Default, Debug)]
pub struct WidgetRender {
    pub spec: OverlaySpec,
    pub diagnostics: Vec<WidgetDiagnostic>,
}

struct Widget {
    path: PathBuf,
    modified: Option<SystemTime>,
    lua: Lua,
    render: Function,
    instructions: Arc<AtomicUsize>,
}

/// Manages independent Lua files. Paths are always explicit; no directories are scanned.
pub struct WidgetManager {
    widgets: Vec<Widget>,
    failed: Vec<(PathBuf, Option<SystemTime>)>,
    diagnostics: Vec<WidgetDiagnostic>,
}

impl WidgetManager {
    pub fn new(paths: &[impl AsRef<Path>]) -> Self {
        let mut manager = Self {
            widgets: Vec::new(),
            failed: Vec::new(),
            diagnostics: Vec::new(),
        };
        for path in paths {
            manager.load(path.as_ref().to_path_buf());
        }
        manager
    }

    pub fn is_empty(&self) -> bool {
        self.widgets.is_empty()
    }

    pub fn render(&mut self, state: &JsonValue) -> WidgetRender {
        let mut result = WidgetRender::default();
        result.diagnostics.append(&mut self.diagnostics);
        let retry: Vec<_> = self
            .failed
            .iter()
            .filter(|(path, old)| fs::metadata(path).and_then(|m| m.modified()).ok() != *old)
            .map(|(p, _)| p.clone())
            .collect();
        for path in retry {
            self.failed.retain(|(p, _)| p != &path);
            self.load(path);
        }
        let mut index = 0;
        while index < self.widgets.len() {
            let path = self.widgets[index].path.clone();
            let changed =
                fs::metadata(&path).and_then(|m| m.modified()).ok() != self.widgets[index].modified;
            if changed {
                self.widgets.remove(index);
                self.load(path);
                continue;
            }
            let widget = &self.widgets[index];
            widget.instructions.store(0, Ordering::Relaxed);
            let value = match widget
                .lua
                .to_value(state)
                .and_then(|input| widget.render.call::<Value>(input))
            {
                Ok(value) => value,
                Err(error) => {
                    result.diagnostics.push(WidgetDiagnostic {
                        path: widget.path.clone(),
                        message: error.to_string(),
                    });
                    index += 1;
                    continue;
                }
            };
            match widget
                .lua
                .from_value::<OverlaySpec>(value)
                .and_then(|spec| {
                    spec.validate().map_err(mlua::Error::external)?;
                    Ok(spec)
                }) {
                Ok(spec) => merge(&mut result.spec, spec),
                Err(error) => result.diagnostics.push(WidgetDiagnostic {
                    path: widget.path.clone(),
                    message: error.to_string(),
                }),
            }
            index += 1;
        }
        result
    }

    fn load(&mut self, path: PathBuf) {
        if self.widgets.len() >= MAX_WIDGETS {
            self.fail(path, "widget limit exceeded".into());
            return;
        }
        let source = match fs::read_to_string(&path) {
            Ok(source) if source.len() <= MAX_SOURCE_BYTES => source,
            Ok(_) => {
                self.fail(path, "widget source exceeds 1 MiB".into());
                return;
            }
            Err(error) => {
                self.fail(path, error.to_string());
                return;
            }
        };
        let lua = match Lua::new_with(
            StdLib::MATH | StdLib::STRING | StdLib::TABLE,
            LuaOptions::default(),
        ) {
            Ok(lua) => lua,
            Err(error) => {
                self.fail(path, error.to_string());
                return;
            }
        };
        if let Err(error) = lua.set_memory_limit(MAX_MEMORY_BYTES) {
            self.fail(path, error.to_string());
            return;
        }
        let instructions = Arc::new(AtomicUsize::new(0));
        let counter = instructions.clone();
        if let Err(error) = lua.set_hook(
            HookTriggers::new().every_nth_instruction(1_000),
            move |_lua, _debug| {
                if counter.fetch_add(1, Ordering::Relaxed) >= INSTRUCTION_LIMIT {
                    Err(mlua::Error::RuntimeError(
                        "widget instruction limit exceeded".into(),
                    ))
                } else {
                    Ok(VmState::Continue)
                }
            },
        ) {
            self.fail(path, error.to_string());
            return;
        }
        let exported = match lua
            .load(&source)
            .set_name(path.to_string_lossy())
            .eval::<Value>()
        {
            Ok(value) => value,
            Err(error) => {
                self.fail(path, error.to_string());
                return;
            }
        };
        let render = match exported {
            Value::Function(function) => Ok(function),
            Value::Table(table) => table.get::<Function>("render"),
            _ => Err(mlua::Error::RuntimeError(
                "widget must return a function or { render = function }".into(),
            )),
        };
        match render {
            Ok(render) => self.widgets.push(Widget {
                modified: fs::metadata(&path).and_then(|m| m.modified()).ok(),
                path,
                lua,
                render,
                instructions,
            }),
            Err(error) => self.fail(path, error.to_string()),
        }
    }

    fn fail(&mut self, path: PathBuf, message: String) {
        let modified = fs::metadata(&path).and_then(|m| m.modified()).ok();
        self.failed.push((path.clone(), modified));
        self.diagnostics.push(WidgetDiagnostic { path, message });
    }
}

fn merge(target: &mut OverlaySpec, source: OverlaySpec) {
    if source.status.is_some() {
        target.status = source.status;
    }
    target.elements.extend(source.elements);
    if target.elements.len() > 4096 {
        target.elements.truncate(4096);
    }
    target.ttl_ms = match (target.ttl_ms, source.ttl_ms) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (None, x) | (x, None) => x,
    };
}

impl std::fmt::Debug for WidgetManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WidgetManager")
            .field("widgets", &self.widgets.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, thread, time::Duration};
    fn file(name: &str, source: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("rhk-{name}-{}.lua", std::process::id()));
        fs::write(&path, source).unwrap();
        path
    }
    #[test]
    fn multiple_widgets_and_no_scripts() {
        let a = file(
            "a",
            "return function(s) return {elements={{type='point',point={x=0.1,y=0.2},color=1}}} end",
        );
        let b = file("b", "return {render=function(s) return {status='ok'} end}");
        let mut m = WidgetManager::new(&[&a, &b]);
        let r = m.render(&JsonValue::Null);
        assert_eq!(r.spec.elements.len(), 1);
        assert_eq!(r.spec.status.as_deref(), Some("ok"));
        assert!(
            WidgetManager::new(&[] as &[&Path])
                .render(&JsonValue::Null)
                .spec
                .elements
                .is_empty()
        );
        let _ = fs::remove_file(a);
        let _ = fs::remove_file(b);
    }
    #[test]
    fn reload_invalid_and_bounded() {
        let path = file("reload", "return function(s) return {status='one'} end");
        let mut m = WidgetManager::new(&[&path]);
        assert_eq!(
            m.render(&JsonValue::Null).spec.status.as_deref(),
            Some("one")
        );
        thread::sleep(Duration::from_millis(5));
        fs::write(&path, "return function(s) while true do end end").unwrap();
        let r = m.render(&JsonValue::Null);
        assert!(r.spec.status.is_none());
        assert!(!r.diagnostics.is_empty());
        let _ = fs::remove_file(path);
    }
}
