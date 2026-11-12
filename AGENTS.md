# Repository guidance

Read README.md and CONTRIBUTING.md before editing. This is a generic public
framework: no user-specific profiles, macros, calibration, target names or
machine paths belong here. Consuming applications own those separately.

Windows builds must target x64. CI and the default configuration use
`x86_64-pc-windows-msvc`; alternate Windows x64 toolchains must pass the same
native checks before producing a runtime drop. Put build and runtime data
outside the repository. Never send live keyboard or mouse input during tests.
