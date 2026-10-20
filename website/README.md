# tacterm website

The website is a small static site published at <https://tacterm.dev/>.
Source lives here on `main`. It builds with Bun using Node-compatible built-ins, without package installation,
third-party fonts, analytics, or runtime requests for content.

The responsive layout supports light, dark, and system themes on every page.
The default follows the operating system, including changes while the page is
open. An explicit choice is saved locally and shared across tabs. The early
`theme.js` script applies it before styles load; CSS also follows the system
when JavaScript is disabled. Storage restrictions do not disable the controls.

The homepage image, `tacterm-windows.png`, is an unedited capture of the native
Windows development build, with a custom color palette and three real tabs:
`workspace`, `checks`, and `README.md`. The active tab shows actual Git history
and a run of the website theme tests. It is not a mockup or a screenshot of the
published release's default configuration. Capture the native window bitmap
(for example, Alt+Print Screen on Windows) and save directly as PNG. Do not
rename or transcode a lossy preview from an automation tool. The build validates
the PNG signature and header, reads its intrinsic dimensions, and versions the
image URL by content hash to invalidate cached captures. The image links to its
full-size version.
Capture helpers and the isolated demo configuration live locally under
`target/site-capture/`, outside the published repository.

Run download metadata and theme behavior checks with:

```sh
bun test ./website/releases.test.mjs ./website/theme.test.mjs
```

## Build

From the repository root, with Bun 1.4.3 (also pinned in the publishing workflow):

```sh
bun website/build.mjs
```

Generated files go to ignored `target/site/`. Do not commit generated output.
To use another URL or destination:

```sh
bun website/build.mjs --url https://example.com/tacterm/ --out-dir target/site
```

Serve the generated directory under the URL's path when previewing locally, so
asset and navigation paths are checked in the same layout as GitHub Pages.

## Content

`FORK_STATUS.md` is the source for the implemented changes, review date, and
release/platform status. The build validates the required sections, renders four
selected highlights on the homepage, and generates `status.html` with user-facing changes and compatibility information
with section navigation. Homepage highlights use concise editorial summaries
in `build.mjs`, each checked against an implemented area in the source document;
review those summaries alongside changes to the underlying feature.
Rust requirements come from `Cargo.toml`. Update the source documents instead of
duplicating version or compatibility details in the homepage. The page leads with
the product, then practical differences, downloads, compatibility questions,
source builds, and the relationship with Rio. Update highlight selection in
`build.mjs` if an area is renamed. The renderer supports the headings, paragraphs,
lists, tables, links, bold text, and inline code used by the status document.

The published build uses `--fetch-release` to read the newest complete tacterm
release (including previews) from GitHub. Its six download links and checksums are
validated against `release.json` and the real attached assets. Downloads are only
shown after publication, never guessed from a version number. API or manifest
errors fail the build rather than replacing a working site with broken links.
For offline previews, supply `--release-file path/to/release.json`. Without either
option the local build links to GitHub Releases without guessing available downloads.

## Publishing

`.github/workflows/pages.yml` builds and deploys on relevant pushes to `main`,
and can also be run manually. Pull requests build without deploying. Builds use
`oven-sh/setup-bun` to install the pinned Bun version; no dependency installation
is needed. GitHub Pages
must use **GitHub Actions** as its publishing source; the legacy `gh-pages` branch
is no longer used. The workflow derives the public URL from GitHub Pages metadata.
The release workflow explicitly dispatches this workflow after publishing, so
download links refresh even when a release is created with the standard GitHub token.
