# tacterm: status and differences from Rio

Last reviewed: **2026-10-20**.

This file describes the changes integrated into tacterm's `main` branch. Update
it when a change is integrated, upstream accepts an equivalent fix, or validation
and release status changes. Proposed changes are not implemented features.

## Upstream baseline

- Upstream: [raphamorim/rio](https://github.com/raphamorim/rio).
- Repository: [narma/tacterm](https://github.com/narma/tacterm).
- Integrated upstream baseline:
  [`59203ccd5f`](https://github.com/raphamorim/rio/commit/59203ccd5fa1e6b760a738c1ad653bb29f2f3465).
  This was also the fetched `upstream/main` tip at this review.
- Main development and integration branch: `main` (formerly `my-fork`).
- Upstream workspace version: `0.5.28`. Independent tacterm release versions are
  committed in [`packaging/VERSION`](packaging/VERSION).

## Implemented changes

| Area | Changes relative to the baseline | Main commits / evidence |
| --- | --- | --- |
| Application icon | Adapt the website's lime `t_` mark for the window icon, Windows EXE resources, macOS app bundles, and Linux desktop integration. Include multiple sizes, Retina representations, and a reproducible vector source. Use a matching Linux app/desktop ID so Wayland can resolve the icon. | [Icon assets and generator](misc/icons/README.md); `frontends/rioterm/build.rs`, `frontends/rioterm/src/router/window.rs`, `packaging/release.mjs` |
| Tab title rendering | Improve mixed Unicode title shaping, fallback font selection, baseline alignment, vertical centering, and pixel alignment. Cache shaped title runs. Support `tab-font-family` and `tab-font-size`, including platform overrides, without changing terminal fallback fonts. | `b7e076cd80`, `24d69c3767`, `a6bc99ab69`, `966f2870f7`, `a99b322020`, `d0252f0ae8`; settings example in [README](README.md#tab-title-fonts) |
| macOS title fonts | Check preferred title font coverage with CoreText, including collection fonts and fonts without backing file data. | `7a67599194`; macOS-specific tests require macOS |
| Configuration reload | Reapply platform overrides when loading an updated configuration, preserving platform-specific settings after reload. | `06b88fb63d` |
| WSL working directories | Open a new tab in the focused tab's directory when inheritance is enabled. Automatically load the WSL zsh working-directory integration; restore the shell directory at prompts and ignore subshell reports. | `126454e0ce`, `fc05f6d77f`, `4ec4cc646e` |
| New tab dimensions | Preserve full window dimensions and the original font size when creating tabs, avoiding repeated subtraction of panel padding and progressive loss of terminal area. | `8dc3e939bd`; context regression test covers repeated tab creation at 150% scale |
| GPU allocation policy | Reduce GPU memory reservations and defer unused atlas storage. | `ebf9ddf67b` |
| Grid rendering resources | Remove unused grid frame buffers and share wgpu grid pipelines between panels in the same window. | `1fcf03d0e9`, `cee1de68d7`; shared-pipeline GPU readback regression test |
| Color glyph atlases | Grow color atlases on demand rather than allocating full-size textures immediately; preserve cached pixels and glyph coordinates during growth. | `be47166333`; [GPU regression tests](sugarloaf/tests/gpu_memory.rs) |
| Grayscale glyph atlases | Start smaller and grow grayscale atlases on demand; refresh bindings when atlas size changes. | `9571ae44a7`; [GPU regression tests](sugarloaf/tests/gpu_memory.rs) |
| Windows timing | Preserve precision for refresh-rate fractions and large QPC counts in the fallback vsync interval calculation. | `d62532facd`; two regression tests |
| Windows VRR presentation | Continuously present focused, visible, non-occluded fullscreen or maximized GPU windows. Reuse resident grid data for idle frames and pace successful frames through the swapchain instead of adding a DWM wait. Fall back to display-paced retries after failed surface acquisition. | `cafbad552c`, `812ae341ab`; unchanged-row and cursor-update GPU readback regression test |

## Performance and limits

GPU memory savings depend on the backend, driver, number of panels, and glyph
workload. Growing atlases retain their capacity after clear, so a heavy workload
can reach the original maximum texture storage. Reduced allocation does not by
itself establish a throughput or latency improvement.

Continuous presentation performs GPU work even when terminal contents are idle;
it trades idle resource use for steady presentation in the specified window
states. Other window states keep event-driven rendering. Behavior still needs
validation across displays and GPUs.

Workstation measurement reports and ad hoc benchmark helpers are kept locally,
outside the published repository. Regression tests remain part of the source.

## Validation

Integration checks on **2026-10-20**, on Windows with Rust stable 1.99.0, passed
against the combined code through merge commit `4500440f8d`:

| Check | Result |
| --- | --- |
| `cargo +stable test -p rio-window --lib --locked -j 4` | 7 passed, including both vsync conversion regression tests |
| `cargo +stable test -p sugarloaf --features wgpu --lib --locked -j 4` | 133 passed, 8 ignored |
| `cargo +stable test -p rioterm --bin rio --locked -j 4` | 228 passed; 1 WSL/zsh integration test ignored |
| Explicit ignored shared-pipeline GPU readback test | 1 passed; covers panel isolation, resize, buffer growth, unchanged-row presentation, and cursor updates |
| `WGPU_BACKEND=dx12` with `cargo +stable test -p sugarloaf --features wgpu --test gpu_memory --locked -j 4 -- --ignored --nocapture` | 2 passed; color and grayscale atlas pixels survive growth and clear on DX12 |
| `cargo +stable fmt --all -- --check`, whitespace, local documentation links | Passed |

The local build/installation scripts also passed PowerShell syntax checks, and
the memory-aware release helper passed `-DryRun`. No release build or installation
was performed for this transition. macOS-only tests and the ignored WSL/zsh
integration test were not executed here.

The subsequent history cleanup removed local benchmark artifacts without
changing application code or GPU regression tests. The retained commits were
checked against their original trees, excluding only the removed artifacts.

The [native project CI](https://github.com/narma/tacterm/actions/runs/39333317119)
also passed formatting, Clippy, and the regular test suite on Windows, macOS,
and Linux for release source commit `e04e22900c`. The separate MSYS2 check is
PR-only and was skipped. Packaging validation (4 tests) and website download
metadata validation (5 tests) passed as well.

The [first release build](https://github.com/narma/tacterm/actions/runs/39333414258)
compiled and packaged all six native OS/architecture targets from `e04e22900c`.
Every packaged executable passed `--version` and `--help`; Linux dependency
resolution and macOS ad-hoc signatures also passed. The downloaded Windows x64
archive and executable matched their recorded SHA256 hashes, and its native
`--version` check reported `tacterm 0.1.0-alpha.1` on the workstation. These are
build, packaging, and CLI checks; no graphical sessions were tested for this release.

For the subsequent application-icon update, the generated assets passed a
regeneration check; the multi-size ICO and 1024 px ICNS decoded successfully.
A local Windows development build passed, and the icon extracted from its EXE
matched the new ICO's opaque pixels and transparent background. macOS Dock and
Linux desktop appearance still require native visual checks.

## Release and compatibility status

The website presentation was refreshed on 2026-10-20 with larger typography,
responsive change and download cards, and light/dark/system theme controls on
the home, status, and error pages. Theme preference persists across navigation;
system mode follows live OS changes. Website metadata and theme behavior checks
passed (9 tests). Browser checks covered light/dark appearance, a 390 px mobile
layout, six release download links, and theme persistence after reload and
navigation. This is a website-only change; it does not change native builds.
The homepage's illustrative terminal was replaced with an actual Windows
development-build screenshot showing three native tabs and a real theme-test
run. The caption identifies the custom theme and development build, and the
image can be opened at full size.
The homepage now explains practical differences before downloads, with
compatibility questions and source-build instructions following them. Concise
editorial summaries link to this complete status document, which includes
section navigation. Builds without release metadata link to GitHub Releases
instead of incorrectly implying that no release exists.

- Release archives package the command as `tacterm` / `tacterm.exe`; macOS uses
  `Tacterm.app` with its own bundle ID. Source builds still produce `rio`, and
  configuration paths and some in-app labels retain Rio compatibility. Settings
  can be shared with an existing Rio installation.
- Independent releases are built by the [tacterm release workflow](.github/workflows/release.yml)
  for Windows, macOS, and Linux, each on native x64 and ARM64 runners.
  [Preview `0.1.0-alpha.1`](https://github.com/narma/tacterm/releases/tag/tacterm-v0.1.0-alpha.1)
  is published with all six archives and SHA256 checksums. The website offers
  downloads validated against the actual GitHub release assets.
- Windows binaries are unsigned; macOS apps are ad-hoc signed and not notarized.
  Windows requires the matching Microsoft Visual C++ v14 Runtime; Linux requires
  its graphics and font libraries in addition to glibc 2.35+.
  Native CLI smoke checks cover version and help output. GUI/GPU sessions and
  older-OS compatibility still require separate validation.
- Existing personal builds and settings remain in place. Public release builds
  do not install or replace the workstation executable.
- Subsequent builds use tacterm's own application icon. The published
  `0.1.0-alpha.1` preview predates this icon change.

## Release automation

- [`packaging/`](packaging/README.md) defines release versions, the native build
  matrix, archive contents, smoke checks, and publishing. Tags use `tacterm-vVERSION`.
  Publishing requires an existing tag pointing to the exact build commit; pushing
  that tag starts the complete workflow automatically.
- Windows uses portable ZIP archives, macOS ZIP archives contain `Tacterm.app`,
  and Linux uses tar.gz archives. Requirements are recorded per artifact.
- All six targets must pass before a release is published. Checks verify artifact
  source/version consistency and SHA256 hashes; a draft is published only after
  all downloads are uploaded. Published versions are not overwritten.
- Releases include `SHA256SUMS`, a machine-readable `release.json`, installation
  instructions, and license notices. The site refreshes automatically after publication.
- The first preview's six public download URLs returned HTTP 200 with matching
  file sizes. All archive hashes agreed between GitHub asset digests, the release
  manifest, and `SHA256SUMS`; the manifest's own checksum also passed.
- Packaging validation tests cover a complete matrix, corrupted archives, mixed
  source commits, and mismatched release tags. Local release-mode CLI compilation
  was checked without building or installing a local Windows release executable.

## Project website

- [tacterm website](https://narma.github.io/tacterm/): an independent project
  homepage with source in [`website/`](website/README.md) on `main`.
- The change count and review date are generated from this document. Homepage
  summaries explain selected implemented changes; the complete technical status
  is published as `status.html`. Download metadata comes from the release manifest.
- A GitHub Actions workflow builds and publishes relevant changes to GitHub
  Pages. It replaces the inherited Rio site previously served from `gh-pages`.
- The site uses static HTML, CSS, and small theme and clipboard scripts, without
  third-party fonts, analytics, or runtime content dependencies.

## Next work

- Evaluate upstream fixes individually, rechecking their current status and
  behavior before integrating them. Proposed changes are not implemented features.
- Add trusted signing and macOS notarization, and define a separate configuration
  migration before promising fully isolated side-by-side settings.
- Continue incorporating upstream changes and keep upstream PRs scoped to one
  problem each.
