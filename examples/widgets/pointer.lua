-- Generic pointer widget. Coordinates must be supplied by the host application.
return function(state)
  if not state.point then return nil end
  return {
    kind = "pointer",
    x = state.point.x,
    y = state.point.y,
    label = state.label or "Target",
    visible = state.visible ~= false
  }
end
