# Lua widgets

`lua::WidgetManager` loads only the Lua files supplied by the consuming
application. Each file returns either `function(state) ... end` or
`{ render = function(state) ... end }`; the returned table is an overlay spec.
Multiple files are evaluated independently and their elements are merged.

The Lua runtime exposes only `math`, `string`, and `table`. Filesystem, process,
module, and debug globals are unavailable. Each render has an instruction
budget; failed or changed widgets contribute no stale visuals and produce a
diagnostic. If no widget loads, the caller receives an empty spec and can avoid
creating an overlay renderer.

For `run` and `rpc` sessions, each widget receives `{engine, state}`. `engine`
is the current status object; `state` is the caller-supplied JSON passed with
`widgets` or `widget_state`. Status includes `armed`, `paused`, `dormant`,
`hotkeys_enabled`, `observe_only`, and held-input counts. Its `rules` array has
each rule's name, enabled state, and `cooldown_remaining_ms`. The
`pending_rules` array reports delayed rules by name and `remaining_ms`, so a
widget can show work waiting to run without predicting which condition will
remain true.

Rule `delay_ms` starts a pending delay after its condition matches. The engine
cancels that pending delay when the condition stops matching, the rule is
disabled, the session pauses, or target presence/focus is lost. Disabling
hotkeys also clears pending rules that depend on key conditions. These status
values are observations; widgets do not control the engine.
