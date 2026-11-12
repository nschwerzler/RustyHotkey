# Script drafts

The CLI exposes the same store:

```text
rhk scripts list
rhk scripts save sample py --file sample.py
rhk scripts read sample py
rhk scripts save sample py --file sample.py --revision REVISION_FROM_READ
```

Pass `--root DIRECTORY` before the subcommand to select a caller-owned directory.
The launcher uses the installed Python SDK, `RHK_PYTHON` and `RHK_PYTHON_PATH`,
just like `rhk ui`.

Open **Scripts** in RHK BOT to create, edit and save drafts. Each application's
workspace lives below its selected data directory, separate from framework
source. Saving never starts a script. Python and JSON syntax errors keep the
editor contents available; Lua drafts are checked when explicitly loaded as
widgets. Reload a changed file before saving after a revision conflict.

The authenticated API exposes `GET /api/scripts`, `GET /api/scripts/NAME.EXT`,
and `POST /api/scripts` with `id`, `extension`, `text` and optional
`expected_revision`. Creation returns 201; stale or missing revisions for an
existing file return 409. Bots can use `list_scripts` and `read_script`;
`save_script` also requires their action-tools setting. Embedded `BotService`
callers opt in with `configure_workspace(root)`; the local app configures it
automatically.

`rustyhotkey.scripts.ScriptWorkspace` stores editable Python, Lua, and JSON
drafts under a caller-provided directory. When no directory is given, it uses
`%APPDATA%/RustyHotkey/scripts`. The workspace is created on construction and
stores only flat files named with a simple identifier and one of `.py`, `.lua`,
or `.json`.

```python
from rustyhotkey.scripts import ScriptWorkspace

workspace = ScriptWorkspace()
saved = workspace.save_script("sample", "py", "value = 1\n")
draft = workspace.read_script(saved["id"], saved["extension"])
workspace.save_script("sample", "py", "value = 2\n", draft["revision"])
```

`list_scripts()` returns metadata; `read_script()` and `save_script()` return
the identifier, extension, text, SHA-256 revision, and `draft_status: "draft"`.
Saving without `expected_revision` creates a new draft and fails if that name
already exists. Updating requires the revision returned by a read or save;
stale revisions raise `ScriptConflict` (`RevisionConflictError`). Writes use a
same-directory temporary file and atomic publication. A workspace lock
serializes revisions between objects and processes. The default file limit is
256,000 UTF-8 bytes and can be lowered or raised up to 1,000,000 bytes. A
workspace holds at most 256 scripts by default; configure `max_scripts` from 1
through 4,096 to change that bound. Listing fails clearly if the workspace
already exceeds its configured count.

Python saves are parsed and compiled to check syntax. This never
imports or executes the draft. JSON is parsed to check JSON syntax. Lua is saved
as bounded UTF-8 text; native Lua validation happens through the runtime when
the caller separately chooses to validate it. Every saved file remains a draft
and this API provides no execution operation.

To run a saved Python revision as a separate operator action, see
[the script runner](script-runner.md). Saving a draft never launches it.
