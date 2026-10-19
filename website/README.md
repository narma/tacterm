# tacterm website

The website is a small static site published at <https://narma.github.io/tacterm/>.
Source lives here on `main`. It uses Node.js built-ins, without package installation,
third-party fonts, analytics, or runtime requests for content.

## Build

From the repository root, with Node.js 24 or later:

```sh
node website/build.mjs
```

Generated files go to ignored `target/site/`. Do not commit generated output.
To use another URL or destination:

```sh
node website/build.mjs --url https://example.com/tacterm/ --out-dir target/site
```

Serve the generated directory under the URL's path when previewing locally, so
asset and navigation paths are checked in the same layout as GitHub Pages.

## Content

`FORK_STATUS.md` is the source for the implemented changes, review date, and
release/platform status. The build validates the required sections, renders four
selected highlights on the homepage, and generates the complete `status.html`.
Rust requirements come from `Cargo.toml`. Update the source documents instead of
duplicating their current status in the homepage. Update highlight selection in
`build.mjs` if an area is renamed. The renderer supports the headings, paragraphs,
lists, tables, links, bold text, and inline code used by the status document.

## Publishing

`.github/workflows/pages.yml` builds and deploys on relevant pushes to `main`,
and can also be run manually. Pull requests build without deploying. GitHub Pages
must use **GitHub Actions** as its publishing source; the legacy `gh-pages` branch
is no longer used. The workflow derives the public URL from GitHub Pages metadata.
