# tacterm application icon

The lime `t_` on a dark green tile comes from `website/favicon.svg`. The app
variant centers the mark, adds transparent outer padding and a subtle edge,
and keeps the original palette. The vector master is `tacterm.svg`.

- `tacterm.ico`: Windows EXE resource, 16/20/24/32/40/48/64/128/256 px.
- `tacterm-256.png`: runtime window icon (Windows and X11).
- `tacterm.icns`: macOS application icon, 16–1024 px and Retina representations.
- `tacterm.svg`: Linux desktop icon; Wayland identifies the installed desktop
  entry by `io.github.narma.tacterm`.
- `tacterm-1024.png`: high-resolution preview.

Generated platform files are committed so builds do not need image tooling.
To regenerate, from the repository root with Node.js and npm:

```sh
npm install --no-save --prefix target/icon-tools sharp@0.35.5
node misc/icons/generate.mjs
node misc/icons/generate.mjs --check
```

`--tools /path/to/tools` can select another directory containing `node_modules/sharp`.
Image tools remain in ignored `target/`. These assets use the repository's MIT license.
