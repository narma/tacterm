# tacterm: changes and compatibility

Last reviewed: **2026-10-20**.

tacterm is an independent terminal based on [Rio](https://github.com/raphamorim/rio).
This page describes implemented changes that affect everyday use, along with
release availability and known limitations.

## Implemented changes

| Area | What changes for you |
| --- | --- |
| Executable name | Launch the terminal as `tacterm` / `tacterm.exe`, or `Tacterm.app` on macOS. |
| Application icon | A distinct lime `t_` icon helps distinguish tacterm from Rio in windows, launchers, and the taskbar or Dock. |
| Tab title rendering | More readable mixed-language tab titles, with improved font fallback and alignment, including macOS font handling. Set `tab-font-family` and `tab-font-size` independently of terminal text fonts; see the [settings example](README.md#tab-title-fonts). |
| Configuration reload | Platform-specific settings remain applied when the configuration reloads. |
| Independent configuration | Settings live in a separate `tacterm` directory, with an optional `TACTERM_CONFIG_HOME` override. On first launch, existing Rio settings and assets are imported if no tacterm configuration exists. Your original Rio configuration is preserved. See [configuration details](README.md#current-status). |
| WSL working directories | With directory inheritance enabled, new tabs open in the focused tab's working directory. WSL zsh integration reports that location automatically. |
| New tab dimensions | Opening more tabs preserves terminal area and font size instead of progressively shrinking the usable space. |
| GPU memory use | Lower initial GPU memory use, including windows with multiple panels. Storage for text and color glyphs grows as needed. |
| Windows VRR presentation | Focused, visible maximized and fullscreen windows keep presenting even when text is idle, for steadier presentation on variable-refresh displays. Refresh timing also handles fractional display rates more accurately. |

## Performance and limits

GPU memory savings vary with the graphics backend, driver, number of panels,
and characters displayed. Memory use can grow with heavier workloads and remain
at that level after clearing the terminal. Lower memory use alone does not imply
faster rendering or lower latency.

Continuous presentation in focused, visible maximized and fullscreen Windows
windows uses GPU resources even while the terminal is idle. Other window states
update on demand. Display and GPU compatibility still needs broader validation.

Rio configuration import has been checked on Windows. Native macOS and Linux
import behavior still needs validation. Graphical behavior and compatibility
with older operating systems also need broader testing.

## Release and compatibility status

- [Version 0.1.0](https://github.com/narma/tacterm/releases/tag/tacterm-v0.1.0)
  is the first non-preview release, with independent settings and the tacterm
  executable name. Downloads are available for Windows, macOS, and Linux on
  x64 and ARM64, with installation instructions and SHA256 checksums.
- Windows downloads are portable ZIP archives. Windows 10 or later is required
  for x64; ARM64 requires Windows 11. Install the matching Microsoft Visual C++
  v14 Runtime. Binaries are unsigned, so Windows may show an unknown-publisher warning.
- macOS downloads contain `Tacterm.app` and require macOS 14 or later. The app
  is ad-hoc signed and not notarized; Gatekeeper may block its first launch.
- Linux downloads are tar.gz archives and require glibc 2.35 or later, graphics
  and font libraries, and a Vulkan-capable driver.
- The earlier `0.1.0-alpha.1` preview shares Rio configuration paths. Version
  0.1.0 uses its own settings directory with one-time import from Rio.

Downloads and an overview of the changes are available at
[tacterm.dev](https://tacterm.dev/).
