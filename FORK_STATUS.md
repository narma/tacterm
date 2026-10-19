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
- Upstream workspace version: `0.5.28`; independent tacterm release versioning
  has not been established.

## Implemented changes

| Area | Changes relative to the baseline | Main commits / evidence |
| --- | --- | --- |
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

## Release and compatibility status

- The repository and documentation use the tacterm name. Executable names,
  application identifiers, package metadata, and configuration paths still use
  Rio's names. Settings are shared with an existing Rio installation.
- Independent tacterm release packaging, signing, update channels, and versioning
  are pending. The inherited release workflow requires adaptation before use.
- Windows is the local validation platform for this integration. Native macOS and
  Linux validation is still needed, including the macOS-specific font tests.
- Existing personal builds and settings remain in place; this repository
  transition does not build or install a replacement executable.

## Project website

- [tacterm website](https://narma.github.io/tacterm/): an independent project
  homepage with source in [`website/`](website/README.md) on `main`.
- Implemented changes, review date, and release/platform status are generated
  from this document. The complete status is also published as `status.html`.
- A GitHub Actions workflow builds and publishes relevant changes to GitHub
  Pages. It replaces the inherited Rio site previously served from `gh-pages`.
- The site uses static HTML, CSS, and a small optional clipboard script, without
  third-party fonts, analytics, or runtime content dependencies.

## Next work

- Evaluate upstream fixes individually, rechecking their current status and
  behavior before integrating them. Proposed changes are not implemented features.
- Adapt packaging and define configuration migration before changing application
  identifiers or distributing tacterm alongside Rio.
- Continue incorporating upstream changes and keep upstream PRs scoped to one
  problem each.
