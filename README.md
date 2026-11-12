# RustyHotkey

A generic, configurable game automation framework in Rust for Windows x64.
Use it as a library or through its command-line and JSON interfaces.

Free for uses permitted by the [PolyForm Noncommercial license](LICENSE.md).
[Commercial use requires a separate agreement](COMMERCIAL.md).

The framework owns reusable automation primitives: target-window selection,
keyboard and mouse input, condition evaluation, pixel matching, screenshot and
OCR access, notifications, recording/replay, and bounded workflows. Applications
supply their own profiles, bindings, calibration and business logic.

Use `rhk simulate SCENARIO.json` to exercise the real engine with scripted
observations and a mock backend; simulation records effects without sending
operating-system input. Live `run` and `rpc` sessions also support
`--observe-only`, which leaves inspection available while refusing arm and
input-producing operations. See [scenario simulation](docs/simulation.md),
[API contracts](docs/api.md), and [capabilities](docs/capabilities.md).

## Build and check

Install stable Rust with the Windows MSVC toolchain and the Visual Studio C++
build tools. The default target is `x86_64-pc-windows-msvc`; Windows builds for
other architectures are rejected.

```powershell
$env:CARGO_TARGET_DIR = Join-Path $env:APPDATA 'RustyHotkey/build'
cargo fmt --check
cargo test --locked
cargo build --release --locked
```

No external automation runtime is required. Native capabilities use Microsoft's
[Rust for Windows](https://github.com/microsoft/windows-rs) bindings. JSON support
uses [Serde](https://serde.rs/) and [serde_json](https://github.com/serde-rs/json).

## Separation of concerns

This repository contains framework code, tests and generic documentation only.
Keep user profiles, recordings, target identities, calibrated coordinates,
private workflows and machine-specific settings in a consuming application or
outside the repository. The framework does not ship game-specific behavior.

See [capabilities](docs/capabilities.md) for implementation status and
[usage](docs/usage.md) for the interfaces. Read [contributing](CONTRIBUTING.md)
before adding capabilities.
