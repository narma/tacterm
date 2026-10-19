# tacterm releases

The independent release version is committed in `packaging/VERSION`. Rust workspace
crate versions continue to describe the Rio base. CI injects `TACTERM_RELEASE_VERSION`
into the application so the packaged `tacterm --version` and terminal version variable
identify the tacterm release. Normal local Rio builds retain their existing version.

## Build and publish

1. Integrate and validate changes on `main`; update `FORK_STATUS.md` and this version file.
2. Push a tag `tacterm-vVERSION` matching the committed version, or manually run the
   **Tacterm release** workflow on `main` with `publish` enabled.
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
desktop entry. Configuration paths and some in-app branding still follow Rio.

Windows binaries are unsigned. macOS apps are ad-hoc signed and not notarized;
Developer ID signing and notarization require separately configured credentials.
Native CLI smoke checks do not replace GUI/GPU or older-OS compatibility testing.
Hosted builds use thin LTO, no debug information, and two compiler jobs to fit runner
memory. Local Windows release builds continue to use the ignored memory-aware helper.

## Checks

```sh
node --test packaging/release.test.mjs
node packaging/release.mjs prepare
```

Generated packages, test fixtures, and reports remain under ignored `target/`.
