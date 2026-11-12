# Operator script runner

`rustyhotkey.script_runner.ScriptRunner` provides an explicit operator action
for launching a saved Python draft. Saving a draft never starts it. Each run
requires the exact revision returned by `ScriptWorkspace.read_script()` or
`save_script()`; stale revisions fail before a child process starts.

In RHK BOT, load a saved `.py` file and select **Run saved Python**. Save any
edits first. The app shows process state, output and a Stop action; closing the
editor keeps the script running, while shutting down the service stops its
managed direct interpreters. The authenticated API uses `GET /api/script-runs`,
`POST /api/script-runs` with `{id,expected_revision}`, and
`POST /api/script-runs/ID/stop` with `{}`. These operations are not registered as
LLM tools. The CLI also supports an explicit foreground run:

```text
rhk scripts run sample --revision REVISION_FROM_READ
```

It prints an initial receipt and a final receipt as JSON lines. Ctrl+C stops
the managed direct interpreter. The final receipt includes bounded stdout and
stderr, and the command fails if the interpreter exits unsuccessfully.

```python
from rustyhotkey.scripts import ScriptWorkspace
from rustyhotkey.script_runner import ScriptRunner

workspace = ScriptWorkspace()
runner = ScriptRunner(workspace)
draft = workspace.read_script("sample", "py")
receipt = runner.run("sample", draft["revision"])
print(receipt["id"], receipt["state"])
```

The runner copies that exact revision to a private snapshot and launches
`sys.executable -B SNAPSHOT` with `shell=False` and stdin closed. It inherits
the current SDK environment, sets `RHK_SCRIPT_PATH` to the editable workspace
file, `RHK_SCRIPT_REVISION` to the approved revision, and `RHK_PYTHON` to the
interpreter path. It appends the workspace to `PYTHONPATH` while preserving
existing entries, and sets `PYTHONIOENCODING=utf-8` so Windows pipe output is
decoded consistently. The child's working directory is the workspace root for
caller-owned assets. Consequently `__file__` points at the immutable run
snapshot while `RHK_SCRIPT_PATH` points at the current editable source. The
snapshot is removed after the child exits.

Use `list_runs()` for receipts, `status(run_id)` to refresh one receipt,
`stop(run_id)` to stop a child, and `close()` during service shutdown. A context
manager also closes the runner. At most four direct Python child processes may
run simultaneously. Stop and close terminate the direct interpreter, wait up
to one second, then kill and wait up to one second if needed. Python code can
start other processes; the runner does not claim to stop those descendants.
The runner retains at most 32 completed receipts, pruning the oldest completed
runs while preserving active runs.

Receipts contain `id`, `script_id`, `revision`, `state` (`running`, `stopping`,
`exited`, or `failed`), `exit_code`, `started_at`, `finished_at`, `output`, and
`output_truncated`. Output is a list of `{stream,text}` chunks from stdout and
stderr, capped at 64 KiB total; excess output is drained and discarded. The
runner never arms RHK or sends input. If a script uses RHK input APIs, those
APIs retain their own target, heartbeat, cancellation, and cleanup rules.

Runs started by RHK BOT automatically forward successful native key and mouse
button edges from their RHK clients to the Keyboard panel. The child receives
a run identifier and an authenticated loopback telemetry endpoint through its
environment. Text contents, pointer positions and screenshots are excluded.
This display bridge never emits input; failure to display events does not
change the script's execution. Finished runs clear their displayed held keys.
