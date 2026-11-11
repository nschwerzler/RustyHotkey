-- Generic status widget. The host supplies state; this script chooses presentation.
return function(state)
  return {
    kind = "overlay",
    anchor = state.anchor or "top_right",
    title = state.title or "Automation",
    message = state.message or "Idle",
    tone = state.tone or "neutral"
  }
end
