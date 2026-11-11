# Contributing

Keep framework mechanisms separate from application policy. Accept target
windows, hotkeys, rules, coordinates and workflow data as explicit inputs.
Avoid fixed game identities, private source material, personal presets and
machine-specific paths in code, examples, tests, documentation and Git history.

Use the Microsoft Windows bindings for native integration. Keep the core engine
independent of native calls so cancellation, rule evaluation and workflows can
be tested with a fake backend. Input should go through the engine's ownership
and cancellation checks. Never inspect or modify another process's memory.

New input-producing paths need tests for cancellation and focus loss. Native
capabilities need non-destructive Windows checks. Do not test by sending input
to a user's application. State whether a check used a fake backend, a test
window or a live application.

Keep builds, captures, profiles, recordings and diagnostics outside the working
tree. Run the checks in the README before committing.

Install the public-content guard once per clone with `git config core.hooksPath
.githooks`. The hook checks staged blobs, and CI checks every tracked blob. Run
`python tools/check_public.py --all` and `python -B -m unittest discover -s tools`
to check manually. Keep local-only marker hashes outside the repository and
point `RHK_PRIVATE_MARKER_HASHES_FILE` at that file; each line is the SHA-256 of
the lowercase alphanumeric identifier. Local markers only add restrictions.
Changes that weaken the shared policy require explicit maintainer approval.
