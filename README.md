# tacterm

A terminal emulator based on [Rio](https://github.com/raphamorim/rio), focused on
performance, simplicity, and practical improvements.

[Website](https://narma.github.io/tacterm/) ·
[Fork status and differences from Rio](FORK_STATUS.md) ·
[Source](https://github.com/narma/tacterm) ·
[Upstream documentation](https://rioterm.com/docs/)

## Raison d’être

tacterm exists to develop and ship the improvements we want in the terminal we
use every day.

Rio provides a strong foundation. This fork gives useful fixes and new ideas an
independent path to release, including changes that remain pending upstream.

Our priorities are straightforward:

- **Performance:** keep input and rendering responsive, and reduce unnecessary
  CPU, GPU, and memory use.
- **Simplicity:** keep everyday use straightforward and the code maintainable.
- **Features with a performance budget:** evaluate new capabilities for their
  effect on latency and resource use.

tacterm will continue incorporating upstream Rio changes and contributing fixes
back where appropriate, while maintaining its own development priorities and
release schedule.

Built on the work of Raphael Amorim and the Rio contributors.

## Current status

The main development and integration branch is `main`. It combines our tab title,
WSL, GPU memory, window sizing, and Windows presentation fixes. See
[FORK_STATUS.md](FORK_STATUS.md) for the implemented changes, evidence, and known
limits; planned work is listed separately there.

Release archives use `tacterm` / `tacterm.exe` and `Tacterm.app`; source builds also
produce `tacterm` / `tacterm.exe`. Some in-app labels retain Rio's names.

Current source builds use their own configuration directory:

- Windows: `%USERPROFILE%\AppData\Local\tacterm`
- macOS: `~/.config/tacterm`
- Linux: `$XDG_CONFIG_HOME/tacterm`, or `~/.config/tacterm`

Set `TACTERM_CONFIG_HOME` to override this directory. When it does not yet exist,
tacterm copies the Rio configuration directory (including themes and other assets,
excluding `log` and `install-backups`) into it. `RIO_CONFIG_HOME` overrides the import source only.
Rio's files remain unchanged; subsequent loads, edits and reloads use tacterm's
copy. An existing tacterm directory, even empty, disables import. If Rio has no
directory, tacterm initializes an empty one and uses defaults.

A failed copy leaves the destination unpublished and reports a configuration
error; it never falls back to reading Rio directly. Linked or special files
require manual migration. Absolute paths inside settings are copied verbatim.
The already published `0.1.0-alpha.1` release still shares Rio's configuration
directory; the isolation above applies to builds containing this change.

The application uses its own lime `t_` icon, adapted from the website, with
native assets for Windows, macOS, and Linux. See [icon sources](misc/icons/README.md).

The upstream Rio installation guides, packages, screenshots, and releases describe
Rio. They are not tacterm distributions. Available tacterm builds, platform
requirements, and signing details are listed on the [website](https://narma.github.io/tacterm/)
and [GitHub Releases](https://github.com/narma/tacterm/releases).

## Building from source

Rio's current minimum supported Rust version is **1.96.1**. With the platform
dependencies installed, build a development executable from the repository root:

```sh
cargo build --locked -p rioterm
```

This produces `target/debug/tacterm` (or `target/debug/tacterm.exe` on Windows). Building
does not install the executable. Consult the
[upstream documentation](https://rioterm.com/docs/) for general configuration and
platform guidance; fork-specific behavior is documented here.

## Tab title fonts

Set a separate font for Rio-rendered tab titles in `config.toml`:

```toml
[navigation]
tab-font-family = "Segoe UI"
tab-font-size = 13
```

The family must be installed on your system. An omitted, empty, or unavailable
family uses the terminal font. The size is in logical pixels, defaults to 12,
and is limited to 8–20 to fit the tab strip. Changes apply when the config is
reloaded and do not change the terminal font. These fields support platform
overrides under `[platform.windows.navigation]` (or `linux`/`macos`).

## Relationship with Rio

Rio remains the upstream project. General fixes intended for Rio are developed
in separate branches so each upstream pull request stays focused. The tacterm
`main` branch integrates the changes intended for this fork.

Rio documentation: [rioterm.com](https://rioterm.com).
Support upstream development: [Sponsor Raphael Amorim](https://github.com/sponsors/raphamorim).

## License

[MIT](LICENSE). Original authorship and license notices are preserved.
