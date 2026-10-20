# tacterm releases

The independent release version is committed in `packaging/VERSION`. Rust workspace
crate versions continue to describe the Rio base. CI injects `TACTERM_RELEASE_VERSION`
into the application so the packaged `tacterm --version` and terminal version variable
identify the tacterm release. Local source builds use the tacterm executable name
and retain the Rio base version unless this variable is supplied.

## Build and publish

1. Integrate and validate changes on `main`; update `FORK_STATUS.md`, this version
   file, and `packaging/release-notes/VERSION.md` with the release's user-facing highlights.
   The publisher uses those highlights before installation and download details.
2. Push a tag `tacterm-vVERSION` matching the committed version. This automatically
   runs the **Tacterm release** workflow. A manual run with `publish` enabled also
   requires that tag to exist and point to the run's exact source commit.
3. Six native jobs build Windows, macOS, and Linux for x64 and ARM64. They package
   the executable, license, installation notes, terminfo source, and release metadata.
4. Every packaged executable must pass `--version` and `--help` on its native host.
   Linux dependencies and macOS ad-hoc signatures are checked as part of packaging.
5. After every job succeeds, the publisher verifies all source/version IDs and
   archive hashes, uploads all six archives plus `release.json` and `SHA256SUMS`
   to a draft, and publishes the complete release. Published assets are immutable:
   use a new version rather than replacing an existing published release.
6. The publisher dispatches the website workflow on `main` to refresh download links.

Leaving `publish` disabled performs native builds and assembly without publishing.
The workflow uses only the normal GitHub token; it does not publish crates, package
registry entries, upstream releases, or overwrite a local installation. The upstream
`.goreleaser.yaml` is not used by this release process.

## Artifacts and requirements

See `targets.json` for runner images, architectures, minimum OS requirements, and
signing status. Windows produces portable ZIPs; macOS ZIPs contain `Tacterm.app` with
its own bundle ID; Linux produces tar.gz archives with `bin/tacterm` and an optional
desktop entry. Configuration uses a separate tacterm directory with one-time
Rio import; some in-app branding still follows Rio.

Application icons come from [`misc/icons/`](../misc/icons/README.md): a multi-size
ICO embedded in the Windows EXE, an ICNS in the macOS bundle, and an SVG in the
Linux icon theme. The Linux desktop entry and application ID both use
`io.github.narma.tacterm` so Wayland can associate the window with its icon.

Windows binaries are unsigned. macOS apps are ad-hoc signed and not notarized;
Developer ID signing and notarization require separately configured credentials.
Native CLI smoke checks do not replace GUI/GPU or older-OS compatibility testing.
Hosted builds use thin LTO, no debug information, and two compiler jobs to fit runner
memory. Local Windows release builds continue to use the ignored memory-aware helper.

Windows builds depend on the Microsoft Visual C++ v14 Redistributable for their
architecture. [Official Microsoft downloads](https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist)
are linked from the website. No third-party runtime installer is bundled or run
automatically. Linux graphics/font runtime dependencies are listed in each archive.

## Checks

```sh
node --test packaging/release.test.mjs
node packaging/release.mjs prepare
```

Generated packages, test fixtures, and reports remain under ignored `target/`.
