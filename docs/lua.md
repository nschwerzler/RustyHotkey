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
