"""Python client for the RustyHotkey JSON-lines control interface."""

from .client import (
    Action, Client, Point, Profile, Region, RustyHotkeyError, RpcError,
)
from .database import Database
from .input_monitor import InputMonitor
from .simulation import simulate
from .scripts import ScriptWorkspace, ScriptWorkspaceError, ScriptConflict
from .script_runner import ScriptRunner, ScriptRunnerError
from .tools import ToolError, ToolRegistry, default_registry

__all__ = ["Action", "Client", "Database", "InputMonitor", "Point", "Profile", "Region", "RpcError", "RustyHotkeyError", "ScriptWorkspace", "ScriptWorkspaceError", "ScriptConflict", "ScriptRunner", "ScriptRunnerError", "ToolError", "ToolRegistry", "default_registry", "simulate"]
