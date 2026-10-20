import { readFile, writeFile, mkdir, copyFile, chmod, stat, rm } from 'node:fs/promises';
import { execFileSync, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { resolve, join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
process.chdir(root);
const targets = JSON.parse(await readFile('packaging/targets.json', 'utf8'));
const version = (await readFile('packaging/VERSION', 'utf8')).trim();
if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z]+(?:[.-][0-9A-Za-z]+)*)?$/.test(version)) throw new Error('Invalid packaging/VERSION');
const tag = `tacterm-v${version}`;
const repo = 'narma/tacterm';
const sha = process.env.GITHUB_SHA || execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
const upstreamVersion = /\[workspace.package\][\s\S]*?version\s*=\s*"([^"]+)"/.exec(await readFile('Cargo.toml', 'utf8'))[1];
const run = (program, args, options = {}) => execFileSync(program, args, { stdio: 'inherit', ...options });
const hashFile = async (path) => createHash('sha256').update(await readFile(path)).digest('hex');
const assetsDir = resolve('target/release-assets');

async function clean() {
  // rust-cache restores target/, including output from earlier packaging runs.
  // Only these generated directories belong to packaging; preserve Cargo output.
  const targetRoot = resolve(root, 'target');
  for (const name of ['release-staging', 'release-assets']) {
    const directory = resolve(targetRoot, name);
    if (dirname(directory) !== targetRoot) throw new Error('Invalid packaging cleanup path');
    await rm(directory, { recursive: true, force: true });
  }
}

function installation(target) {
  const intro = `# tacterm ${version}\n\nPlatform: ${target.id}\nSource: https://github.com/${repo}/commit/${sha}\nRio base version: ${upstreamVersion}\nMinimum: ${target.minimum}\nSigning: ${target.signing}\n\n`;
  const common = `\n## Settings and compatibility\n\ntacterm uses its own configuration directory and TACTERM_CONFIG_HOME override.\nWindows: %USERPROFILE%\\AppData\\Local\\tacterm\\config.toml.\nmacOS/Linux: ~/.config/tacterm/config.toml (Linux respects XDG_CONFIG_HOME).\nOn first use, if the tacterm directory is absent, Rio settings and assets are\ncopied into it, excluding logs and installer backups. RIO_CONFIG_HOME overrides only the import source.\nExisting tacterm directories are never reimported. Rio files remain unchanged;\nthere is no runtime fallback to Rio. Linked files require manual migration.\nAbsolute paths within settings are preserved. Extraction alone changes no settings.\n\nTerminal integration identifiers and some in-app labels still come from Rio.\nThe packaged command is tacterm; --version reports this tacterm release version.\n\n## Verification\n\nCompare this archive's SHA256 with SHA256SUMS on its GitHub release page.\nrelease-info.json records the version, source commit, and packaged executable hash.\nNative CI checks --version and --help; these checks do not validate a graphical\nterminal session, GPU rendering, or every supported OS version.\n\nMIT license; based on Rio by Raphael Amorim and the Rio contributors.\n`;
  if (target.platform === 'windows') return intro + `## Run\n\nExtract this entire ZIP and run tacterm.exe. Windows may show an unknown-publisher\nwarning because these preview binaries are not Authenticode-signed.\nThe archive is portable and does not change PATH or install over an existing Rio.\n\nInstall the current Microsoft Visual C++ v14 Redistributable for this architecture\nif it is not already present, especially if Windows reports MSVCP140.dll or\nVCRUNTIME140.dll missing. Official downloads:\nhttps://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist\n` + common;
  if (target.platform === 'macos') return intro + `## Install\n\nExtract the ZIP and move Tacterm.app to Applications, then open it. This preview has\nan ad-hoc signature, not a Developer ID signature, and is not notarized by Apple.\nGatekeeper may block it. If you choose to trust this build after verifying its source\nand checksum, follow Apple's per-app Open Anyway instructions:\nhttps://support.apple.com/102445\nDo not disable Gatekeeper system-wide.\n\nCLI: /Applications/Tacterm.app/Contents/MacOS/tacterm --version\n` + common;
  return intro + `## Run / install for your user\n\nRun ./bin/tacterm from this extracted directory, or copy bin/tacterm to a directory\non your PATH (for example ~/.local/bin). No root installer is included.\nThe optional desktop entry is in share/applications and its icon in share/icons.\nThe terminfo source in extras/rio.terminfo is optional; Rio falls back to xterm-256color\nwhen its own terminfo is not installed.\n\nInstall runtime fontconfig, FreeType, X11/Wayland, xkbcommon, and a working Vulkan\ndriver through your distribution. On Ubuntu 22.04 the usual runtime packages include\nlibfontconfig1 libfreetype6 libx11-6 libxcb1 libxkbcommon0 libwayland-client0 libvulkan1.\n` + common;
}

async function prepare() {
  if (process.env.GITHUB_REF_TYPE === 'tag' && process.env.GITHUB_REF_NAME !== tag) throw new Error(`Tag must match committed version: ${tag}`);
  if (process.env.PUBLISH === 'true') {
    const tagged = execFileSync('gh', ['api', `repos/${repo}/commits/${tag}`, '--jq', '.sha'], { encoding: 'utf8' }).trim();
    if (tagged !== sha) throw new Error('Push the matching release tag for this exact source commit before publishing');
  }
  const output = `version=${version}\ntag=${tag}\nprerelease=${version.includes('-')}\npublish=${process.env.PUBLISH === 'true'}\nmatrix=${JSON.stringify({ include: targets })}\n`;
  if (process.env.GITHUB_OUTPUT) await writeFile(process.env.GITHUB_OUTPUT, output, { flag: 'a' });
  console.log(output);
}

async function pack(id) {
  const target = targets.find((item) => item.id === id);
  if (!target) throw new Error(`Unknown release target: ${id}`);
  const host = { win32: 'windows', darwin: 'macos', linux: 'linux' }[process.platform];
  if (host !== target.platform || process.arch !== (target.arch === 'x64' ? 'x64' : 'arm64')) throw new Error('Package must be built and smoke-tested on its native OS/architecture');
  const stem = `tacterm-${version}-${id}${target.platform === 'windows' ? '-portable' : ''}`;
  const stageParent = resolve('target/release-staging');
  const stage = join(stageParent, stem);
  try { await stat(stage); throw new Error('Staging directory already exists; use a fresh build checkout'); } catch (error) { if (error.code !== 'ENOENT') throw error; }
  await mkdir(stage, { recursive: true });
  await mkdir(assetsDir, { recursive: true });
  const source = resolve('target', target.target, 'release', target.platform === 'windows' ? 'tacterm.exe' : 'tacterm');
  let executable;
  if (target.platform === 'macos') {
    const app = join(stage, 'Tacterm.app');
    const contents = join(app, 'Contents');
    await mkdir(join(contents, 'MacOS'), { recursive: true });
    await mkdir(join(contents, 'Resources'), { recursive: true });
    executable = join(contents, 'MacOS/tacterm');
    await copyFile(source, executable);
    await chmod(executable, 0o755);
    await copyFile('misc/icons/tacterm.icns', join(contents, 'Resources/icon.icns'));
    let plist = await readFile('misc/osx/Rio.app/Contents/Info.plist', 'utf8');
    plist = plist.replaceAll('Rio', 'Tacterm').replaceAll('{{.Version}}', version.split('-')[0]).replace(/\{\{\.Now.Format "[^"]+"\}\}/g, process.env.GITHUB_RUN_NUMBER || '1');
    const plistPath = join(contents, 'Info.plist');
    await writeFile(plistPath, plist);
    for (const [key, value] of Object.entries({ CFBundleName: 'Tacterm', CFBundleDisplayName: 'Tacterm', CFBundleExecutable: 'tacterm', CFBundleIdentifier: 'io.github.narma.tacterm', CFBundleVersion: process.env.GITHUB_RUN_NUMBER || '1', LSMinimumSystemVersion: '14.0' })) run('/usr/libexec/PlistBuddy', ['-c', `Set :${key} ${value}`, plistPath]);
    for (const key of ['CFBundleURLTypes', 'CFBundleDocumentTypes', 'NSServices']) run('/usr/libexec/PlistBuddy', ['-c', `Delete :${key}`, plistPath]);
    run('plutil', ['-lint', plistPath]);
    run('codesign', ['--force', '--sign', '-', app]);
    run('codesign', ['--verify', '--strict', '--verbose=2', app]);
  } else {
    const binaryDirectory = target.platform === 'windows' ? stage : join(stage, 'bin');
    await mkdir(binaryDirectory, { recursive: true });
    executable = join(binaryDirectory, target.platform === 'windows' ? 'tacterm.exe' : 'tacterm');
    await copyFile(source, executable);
    if (target.platform === 'linux') {
      await chmod(executable, 0o755);
      const iconDir = join(stage, 'share/icons/hicolor/scalable/apps');
      await mkdir(iconDir, { recursive: true });
      await copyFile('misc/icons/tacterm.svg', join(iconDir, 'tacterm.svg'));
      await mkdir(join(stage, 'share/applications'), { recursive: true });
      await writeFile(join(stage, 'share/applications/io.github.narma.tacterm.desktop'), '[Desktop Entry]\nType=Application\nName=tacterm\nComment=A terminal based on Rio\nExec=tacterm --app-id io.github.narma.tacterm\nIcon=tacterm\nStartupWMClass=io.github.narma.tacterm\nTerminal=false\nCategories=System;TerminalEmulator;\n');
      const dependencies = execFileSync('ldd', [executable], { encoding: 'utf8' });
      if (dependencies.includes('not found')) throw new Error('Packaged binary has missing shared libraries');
      console.log(dependencies);
    }
  }
  const versionOutput = execFileSync(executable, ['--version'], { encoding: 'utf8', timeout: 30000 });
  if (!versionOutput.includes(version)) throw new Error(`Incorrect packaged version: ${JSON.stringify(versionOutput)}`);
  const help = execFileSync(executable, ['--help'], { encoding: 'utf8', timeout: 30000 });
  if (!help.includes('--working-dir')) throw new Error('Packaged CLI help is incomplete');
  console.log(`Native CLI smoke check passed: ${versionOutput.trim()}`);
  await copyFile('LICENSE', join(stage, 'LICENSE'));
  // Preserve the license notices shipped with the native workspace and bundled font.
  for (const license of [
    'corcovado/LICENSE', 'corcovado/ORIGINAL_LICENSE', 'rio-window/LICENSE',
    'rio-grapheme-width/LICENSE', 'rio-backend/LICENSE', 'sugarloaf/LICENSE',
    'teletypewriter/LICENSE', 'frontends/rioterm/LICENSE',
    'sugarloaf/src/font/resources/CascadiaCode/LICENSE',
  ]) {
    const destination = join(stage, 'licenses', license);
    await mkdir(dirname(destination), { recursive: true });
    await copyFile(license, destination);
  }
  await writeFile(join(stage, 'README.md'), installation(target));
  await mkdir(join(stage, 'extras'), { recursive: true });
  await copyFile('misc/rio.terminfo', join(stage, 'extras/rio.terminfo'));
  const info = { version, tag, commit: sha, upstreamVersion, ...target, binarySha256: await hashFile(executable), smoke: ['--version', '--help'], guiTested: false };
  await writeFile(join(stage, 'release-info.json'), JSON.stringify(info, null, 2) + '\n');
  const archive = `${stem}.${target.format}`;
  const archivePath = join(assetsDir, archive);
  if (target.platform === 'macos') run('ditto', ['-c', '-k', '--keepParent', stage, archivePath]);
  else run('tar', [target.format === 'zip' ? '-a' : '-z', '-cf', archivePath, '-C', stageParent, stem]);
  const archiveListing = execFileSync('tar', ['-tf', archivePath], { encoding: 'utf8' });
  if (!archiveListing.includes('release-info.json') || !archiveListing.includes('LICENSE')) throw new Error('Incomplete release archive');
  await writeFile(join(assetsDir, `build-${id}.json`), JSON.stringify({ ...info, archive, archiveSha256: await hashFile(archivePath), bytes: (await stat(archivePath)).size }, null, 2) + '\n');
  console.log(`Packaged: ${archive}`);
}

async function setup(id) {
  const target = targets.find((item) => item.id === id && item.platform === 'windows');
  if (!target) throw new Error(`Unknown Windows target: ${id}`);
  const portable = JSON.parse(await readFile(join(assetsDir, `build-${id}.json`), 'utf8'));
  const archive = `tacterm-${version}-${id}-setup.exe`;
  const path = join(assetsDir, archive);
  await writeFile(join(assetsDir, `setup-${id}.json`), JSON.stringify({
    ...portable, archive, archiveSha256: await hashFile(path), bytes: (await stat(path)).size,
    format: 'exe', kind: 'setup',
  }, null, 2) + '\n');
}

async function assemble() {
  const records = [];
  for (const target of targets) {
    const record = JSON.parse(await readFile(join(assetsDir, `build-${target.id}.json`), 'utf8'));
    if (record.version !== version || record.commit !== sha || record.id !== target.id) throw new Error('Artifacts do not come from the same release commit and version');
    const expected = `tacterm-${version}-${target.id}${target.platform === 'windows' ? '-portable' : ''}.${target.format}`;
    if (record.archive !== expected) throw new Error(`Unexpected archive name: ${target.id}`);
    if (await hashFile(join(assetsDir, record.archive)) !== record.archiveSha256) throw new Error(`Archive checksum mismatch: ${target.id}`);
    records.push({ ...record, kind: target.platform === 'windows' ? 'portable' : 'archive' });
    if (target.platform === 'windows') {
      const installer = JSON.parse(await readFile(join(assetsDir, `setup-${target.id}.json`), 'utf8'));
      if (installer.version !== version || installer.commit !== sha || installer.id !== target.id ||
          installer.platform !== target.platform || installer.arch !== target.arch ||
          !record.binarySha256 || installer.binarySha256 !== record.binarySha256 ||
          installer.archive !== `tacterm-${version}-${target.id}-setup.exe`) {
        throw new Error(`Installer does not match portable build: ${target.id}`);
      }
      if (await hashFile(join(assetsDir, installer.archive)) !== installer.archiveSha256) throw new Error(`Installer checksum mismatch: ${target.id}`);
      records.push({ ...installer, kind: 'setup' });
    }
  }
  const manifest = { schemaVersion: 2, version, tag, commit: sha, upstreamVersion, prerelease: version.includes('-'), assets: records.map(({ archive, archiveSha256, platform, arch, minimum, runtime, signing, bytes, kind }) => ({ name: archive, sha256: archiveSha256, platform, arch, minimum, runtime, signing, bytes, kind, url: `https://github.com/${repo}/releases/download/${tag}/${archive}` })) };
  await writeFile(join(assetsDir, 'release.json'), JSON.stringify(manifest, null, 2) + '\n');
  const checksumNames = [...records.map((record) => record.archive), 'release.json'];
  await writeFile(join(assetsDir, 'SHA256SUMS'), (await Promise.all(checksumNames.map(async (name) => `${await hashFile(join(assetsDir, name))}  ${name}`))).join('\n') + '\n');
  return manifest;
}

async function publish() {
  const manifest = await assemble();
  const existing = spawnSync('gh', ['release', 'view', tag, '--repo', repo, '--json', 'isDraft'], { encoding: 'utf8' });
  if (existing.status === 0 && !JSON.parse(existing.stdout).isDraft) throw new Error('This version is already published. Bump packaging/VERSION instead of replacing its assets.');
  const tagCommit = spawnSync('gh', ['api', `repos/${repo}/commits/${tag}`, '--jq', '.sha'], { encoding: 'utf8' });
  if (tagCommit.status !== 0 || tagCommit.stdout.trim() !== sha) throw new Error('A matching release tag must already point to this exact commit');
  const highlights = (await readFile(resolve("packaging", "release-notes", `${version}.md`), "utf8")).trim();
  if (!highlights) throw new Error("Release highlights are required");
  const notes = `# tacterm ${version}\n\n${highlights}\n\nSource: https://github.com/${repo}/commit/${sha}\nChanges: https://github.com/${repo}/blob/${sha}/FORK_STATUS.md\n\n## Downloads\n\nWindows portable ZIPs and Setup installers, macOS Tacterm.app ZIPs, and Linux tar.gz archives are attached for x64 and ARM64. Each target is compiled on its native OS/architecture and checked with --version and --help. GUI/GPU sessions are not validated by these smoke checks.\n\n- Windows binaries are **unsigned** and require the Microsoft Visual C++ v14 Redistributable for the matching architecture: https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist\n- macOS apps have an **ad-hoc signature and are not notarized**. See Apple's per-app opening instructions: https://support.apple.com/102445\n- macOS requires 14+, Linux requires glibc 2.35+ and graphics/font libraries.\n- The packaged command is tacterm. Settings use an independent tacterm directory with one-time import from Rio; some internal branding remains Rio-compatible.\n\nEach archive contains its installation README, MIT license, and release-info.json. Check SHA256SUMS before installing. Portable archives do not modify your existing installation. Windows Setup defaults to the current user without elevation, offers an all-users mode, and adds a Start menu shortcut. Desktop shortcut creation and launch after setup are optional; uninstalling preserves user settings.\n`;
  const notesPath = resolve('target/release-notes.md');
  await writeFile(notesPath, notes);
  // The verified existing tag fixes the source. Leave target_commitish at its
  // default (main): targeting an older SHA after workflow edits requires a
  // Workflows-write permission unavailable to GITHUB_TOKEN.
  if (existing.status !== 0) run('gh', ['release', 'create', tag, '--repo', repo, '--verify-tag', '--title', `tacterm ${version}`, '--draft', ...(manifest.prerelease ? ['--prerelease'] : []), '--notes-file', notesPath]);
  else run('gh', ['release', 'edit', tag, '--repo', repo, '--notes-file', notesPath]);
  run('gh', ['release', 'upload', tag, '--repo', repo, ...manifest.assets.map((asset) => join(assetsDir, asset.name)), join(assetsDir, 'release.json'), join(assetsDir, 'SHA256SUMS'), '--clobber']);
  const uploaded = JSON.parse(execFileSync('gh', ['release', 'view', tag, '--repo', repo, '--json', 'assets'], { encoding: 'utf8' }));
  for (const asset of manifest.assets) {
    const remote = uploaded.assets.find((item) => item.name === asset.name);
    if (!remote || remote.size !== asset.bytes) throw new Error(`Incomplete uploaded release: ${asset.name}`);
    if (remote.digest && remote.digest !== `sha256:${asset.sha256}`) throw new Error(`Uploaded checksum mismatch: ${asset.name}`);
  }
  run('gh', ['release', 'edit', tag, '--repo', repo, '--draft=false', ...(manifest.prerelease ? ['--prerelease', '--latest=false'] : ['--prerelease=false', '--latest'])]);
  console.log(`Published https://github.com/${repo}/releases/tag/${tag}`);
}

const [command, id] = process.argv.slice(2);
if (command === 'prepare') await prepare();
else if (command === 'clean') await clean();
else if (command === 'pack') await pack(id);
else if (command === 'setup') await setup(id);
else if (command === 'assemble') await assemble();
else if (command === 'publish') await publish();
else throw new Error('Usage: node packaging/release.mjs prepare | clean | pack TARGET_ID | setup TARGET_ID | assemble | publish');
