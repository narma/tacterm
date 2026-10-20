# AGENTS.md — tacterm

## Project and sources of truth

tacterm is a terminal emulator forked from Rio. The desktop binary is `tacterm`
(`tacterm.exe` on Windows); its Cargo package is still named `rioterm`.
The UI uses Sugarloaf and rio-window, not GPUI. rio-window is a winit fork;
Windows rendering requires the `wgpu` feature.

Read the relevant crate manifests and nearby code before changing a subsystem.
Use these entry points rather than assuming another project's architecture:

- `frontends/rioterm/`: desktop application, event handling and terminal UI.
- `rio-backend/`: terminal backend and configuration.
- `sugarloaf/`, `rio-graphics/`, `rio-fonts/`: rendering and fonts.
- `rio-window/`: window system and platform event integration.
- `teletypewriter/`, `corcovado/`: PTY and I/O infrastructure.
- `rio-vt/`, `rio-grid/`, `rio-unicode/`, `rio-grapheme-width/`: terminal data and text processing.
- `packaging/` and `.github/workflows/release.yml`: release assembly and validation.
- `website/`: static site, including download manifest validation.

`Cargo.toml` defines workspace membership and the Rust requirement.
`packaging/VERSION` defines the independent tacterm release version; workspace
crate versions continue to identify the Rio base. Do not change both by habit.

## Integration and Git

`main` is the development and personal integration branch, formerly `my-fork`.
Keep changes intended for installed builds integrated there, including work
completed on feature or PR branches. Keep each upstream PR limited to its scope.

- `origin`: https://github.com/narma/tacterm.git
- `upstream`: https://github.com/raphamorim/rio.git

Inspect the working tree and current branch before editing. Preserve unrelated
changes. Stage explicit paths and review the staged diff before committing.
Do not rewrite published history or force-push without an explicit request.
For issues and PRs, inspect the destination repository's current templates and
labels; use them when available. An upstream PR must exclude tacterm-only branding,
packaging, website and workstation changes unless they are its intended subject.

This `AGENTS.md` is tracked and public. Keep personal paths and workstation-only
instructions in ignored files. If `target/agent-instructions/workstation.md`
exists, read it before local Windows release builds or installation. It supplies
machine-specific defaults and does not authorize installation on its own.

## Documentation scope

By default, `FORK_STATUS.md` records only significant user-facing differences
from upstream Rio. Do not add routine integration history, CI progress, validation
logs or packaging implementation details unless explicitly requested.

README files explain current usage and necessary setup. Keep them concise;
correct outdated instructions in place instead of appending a changelog for every
change. Release history belongs in `packaging/release-notes/`.

The website consumes `FORK_STATUS.md`. When changing its structure or links,
check the generated site. Do not announce unpublished downloads as available.
Use repository-relative links in public documentation, not workstation paths.

## Implementation principles

Prefer focused changes in the responsible crate. Share behavior at existing
boundaries; avoid spreading platform checks through unrelated rendering code.
Follow surrounding module conventions rather than imposing a new layout.

Use descriptive names and comments for constraints or rationale. Propagate
recoverable errors with context; handle or log intentional failures explicitly.
Avoid new panic paths for user input, terminal output or configuration. Document
invariants around unsafe code and validate buffer sizes before access.

Keep blocking work off the event loop. Give background work a clear lifetime
and a way to report failure. Avoid repeated allocation, font discovery and other
expensive work per frame; invalidate caches when their inputs change. Measure
performance claims against comparable workloads rather than assuming a win.

Preserve keyboard access and focus behavior when changing controls. Check mouse
and keyboard interaction together, including tab/pane switching and window focus.

## Terminal-specific compatibility

- Keep PTY, shell integration and escape-sequence behavior compatible unless the
  requested change requires otherwise. Treat shell output and titles as input,
  not commands to execute.
- Text changes must account for Unicode graphemes, character width, fallback
  fonts and mixed-language titles. Avoid conflating glyph count with cell width.
- Keep tab-font settings independent from terminal-font settings. Check DPI,
  resizing and terminal area preservation when changing layout.
- Preserve platform-specific configuration on reload. Use tacterm's own config
  directory and `TACTERM_CONFIG_HOME`; do not restore a runtime fallback to Rio.
  First-use import must preserve Rio files and existing tacterm settings.
- Test graphics changes on the affected backend. In particular, changes to redraw
  scheduling must preserve the intended focused/maximized/fullscreen Windows VRR
  behavior and event-driven behavior in other states.

## Validation

Choose checks for the affected behavior. Add regression tests for bugs and new
logic; do not add tests merely to restate a documentation or cosmetic edit.
Keep regression tests such as `sugarloaf/tests/gpu_memory.rs` tracked.

Useful commands from the repository root:

```sh
cargo check -p rioterm --features wgpu
cargo build -p rioterm --bin tacterm --features wgpu
cargo fmt --all -- --check
cargo clippy --all-targets --all-features
cargo test --features wgpu
node --test packaging/release.test.mjs website/releases.test.mjs website/theme.test.mjs
node website/build.mjs --out-dir target/site
```

Start with targeted crate or test filters for a small Rust change. Use the current
CI workflows for the full platform checks and dependency setup. If using nextest,
remember that doctests require a separate Cargo invocation. Report failures and
unavailable platform checks accurately; a compile or CLI check does not validate
rendering or an interactive terminal session.

For UI changes, reproduce the affected interaction in a running build and inspect
visual output where possible. Create a temporary tacterm configuration directory
before launching and point `TACTERM_CONFIG_HOME` at it to avoid importing or
changing personal settings. Use disposable shell sessions; do not interrupt the
user's existing terminals. Store test captures and measurements under `target/`.

## Builds, packaging and installation

For local Windows release builds on the configured workstation, use the ignored
`build-release.ps1` helper, not an unrestricted `cargo build --release`:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\build-release.ps1 -DryRun
powershell -NoProfile -ExecutionPolicy Bypass -File .\build-release.ps1
```

The helper reserves memory for LTO and the system; its estimate is not a hard
limit. It restores process environment variables afterward. If it is absent on a
fresh checkout, use the documented CI build process or establish an appropriate
local memory budget before a release build; do not invent a missing helper.

Build Rust once per target. Windows portable and Setup packages must consume the
same staged executable. Preserve architecture checks, native CLI smoke checks,
installed-versus-portable SHA256 comparison, complete manifests and checksums.
Keep release assembly and website validation compatible when asset naming changes.

Use `publish=false` for release-workflow validation unless publication is requested.
Do not replace an existing published release's assets; follow `packaging/README.md`
for versioning and publishing. A successful push is not a successful CI run:
report the run URL and distinguish running, failed and completed validation.

A build request does not imply permission to install. When installation is
requested, build from integrated `main`, use the workstation's configured installer
if present, stage and verify the executable, back up the installed version, and
verify the replacement. Restore the backup if replacement fails. Preserve legacy
Rio executables and configuration unless removal is explicitly requested. Never
force-close terminal sessions to replace a running executable.

## Local artifacts

Keep workstation measurement reports, research notes, ad hoc benchmark helpers,
installation scripts and backups under ignored `target/` directories. Do not
publish them or reintroduce them into Git history. Preserve existing local
benchmark backups. This restriction does not apply to regression tests or
intentional product assets already tracked in the repository.
