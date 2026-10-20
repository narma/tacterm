import { readFile, writeFile, mkdir, copyFile } from 'node:fs/promises';
import { resolve, dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { loadRelease } from './releases.mjs';

const sourceDir = dirname(fileURLToPath(import.meta.url));
const root = resolve(sourceDir, '..');
const args = process.argv.slice(2);
function option(name, fallback) {
  const index = args.indexOf(name);
  if (index < 0) return fallback;
  if (!args[index + 1] || args[index + 1].startsWith('--')) throw new Error(`Missing ${name}`);
  return args[index + 1];
}
const outDir = resolve(root, option('--out-dir', 'target/site'));
if (outDir === root || outDir === sourceDir) throw new Error('Use a separate output directory');
const siteUrl = new URL(option('--url', process.env.SITE_URL || 'https://narma.github.io/tacterm/'));
if (!['http:', 'https:'].includes(siteUrl.protocol)) throw new Error('Expected an HTTP site URL');
siteUrl.search = '';
siteUrl.hash = '';
siteUrl.pathname = `${siteUrl.pathname.replace(/\/+$/, '')}/`;
const base = siteUrl.pathname;
const repo = 'https://github.com/narma/tacterm';
const status = await readFile(join(root, 'FORK_STATUS.md'), 'utf8');
const cargo = await readFile(join(root, 'Cargo.toml'), 'utf8');
const escape = (value) => String(value).replace(/[&<>"']/g, (char) => ({
  '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;',
}[char]));
const release = await loadRelease({ file: option('--release-file', null), online: args.includes('--fetch-release') });
const screenshot = await readFile(join(sourceDir, 'tacterm-windows.png'));
if (screenshot.length < 33 ||
    !screenshot.subarray(0, 8).equals(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10])) ||
    screenshot.readUInt32BE(8) !== 13 || screenshot.toString('ascii', 12, 16) !== 'IHDR') {
  throw new Error('tacterm-windows.png must be a native lossless PNG capture, not a renamed JPEG');
}
const screenshotWidth = screenshot.readUInt32BE(16);
const screenshotHeight = screenshot.readUInt32BE(20);
if (!screenshotWidth || !screenshotHeight) throw new Error('Invalid screenshot dimensions');
const screenshotVersion = createHash('sha256').update(screenshot).digest('hex').slice(0, 12);

function downloadSection() {
  if (!release) return `<section id="downloads" class="downloads-section wrap" aria-labelledby="downloads-title"><p class="eyebrow">RELEASE BUILDS</p><h2 id="downloads-title">Get tacterm.</h2><p class="download-intro">Release metadata is not included in this build of the site. Visit <a href="https://github.com/narma/tacterm/releases">GitHub Releases</a> for available downloads, or <a href="#build">build from source</a>.</p><a class="text-link" href="${repo}/releases">View GitHub Releases ↗</a></section>`;
  const labels = { windows: 'Windows', macos: 'macOS', linux: 'Linux' };
  const notes = { windows: 'Portable ZIP · Windows 10+ (x64), Windows 11 (ARM64). Requires Microsoft Visual C++ v14 Runtime. Unsigned binaries; Windows may show an unknown-publisher warning.', macos: 'Tacterm.app · macOS 14+. Ad-hoc signed; not notarized. Gatekeeper may block the first launch.', linux: 'tar.gz · glibc 2.35+ (Ubuntu 22.04+). Requires font libraries and a Vulkan-capable driver.' };
  const cards = Object.entries(labels).map(([platform, label]) => {
    const preferredArch = platform === 'macos' ? 'arm64' : 'x64';
    const links = release.assets.filter((asset) => asset.platform === platform).sort((a, b) => Number(b.arch === preferredArch) - Number(a.arch === preferredArch)).map((asset) => {
      const arch = platform === 'macos' ? (asset.arch === 'arm64' ? 'Apple Silicon' : 'Intel') : (asset.arch === 'x64' ? 'x64' : 'ARM64');
      return `<a class="download-link" aria-label="Download ${label} ${arch}" href="${escape(asset.url)}"><span>${arch}</span><span>${(asset.bytes / 1048576).toFixed(1)} MB &nbsp; ↓</span></a>`;
    }).join('');
    return `<article class="download-card"><h3>${label}</h3>${links}<p>${notes[platform]}</p>${platform === 'windows' ? '<a class="text-link" href="https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist">Microsoft runtime downloads ↗</a>' : ''}</article>`;
  }).join('');
  return `<section id="downloads" class="downloads-section wrap" aria-labelledby="downloads-title"><div class="section-heading"><div><p class="eyebrow">${release.prerelease ? 'PREVIEW RELEASE' : 'RELEASE BUILDS'} / ${escape(release.version)}</p><h2 id="downloads-title">Choose your platform.</h2></div><a class="text-link" href="${escape(release.releaseUrl)}">Release notes &amp; installation ↗</a></div><p class="download-intro">Download the archive for your system and processor, then follow the installation notes included inside.</p><div class="download-notice"><strong>Already use Rio?</strong> Version 0.1.0 imports Rio settings into a separate tacterm directory on first launch. The earlier 0.1.0-alpha.1 preview shared Rio settings. <a href="#questions">Read the compatibility notes ↓</a></div><div class="download-grid">${cards}</div><div class="download-footer"><a class="text-link" href="${escape(release.checksumUrl)}">SHA256 checksums ↗</a><a class="text-link" href="${escape(release.releaseUrl)}">All assets on GitHub ↗</a><span>Build and command-line checks passed; graphical compatibility varies. <a href="${escape(base)}status.html#validation">See validation details</a>.</span></div></section>`;
}

function sourceLink(value) {
  if (value.startsWith('#')) return value;
  if (/^https?:\/\//.test(value)) return value;
  if (/^[a-z][a-z0-9+.-]*:/i.test(value)) throw new Error(`Unsupported Markdown link: ${value}`);
  return `${repo}/blob/main/${value}`;
}

function inline(value) {
  const pattern = /`([^`]+)`|\[([^\]]+)\]\(([^)]+)\)|\*\*([^*]+)\*\*/g;
  let html = '';
  let position = 0;
  for (const match of value.matchAll(pattern)) {
    html += escape(value.slice(position, match.index));
    if (match[1]) html += `<code>${escape(match[1])}</code>`;
    else if (match[2]) html += `<a href="${escape(sourceLink(match[3]))}">${inline(match[2])}</a>`;
    else html += `<strong>${escape(match[4])}</strong>`;
    position = match.index + match[0].length;
  }
  return html + escape(value.slice(position));
}

function bulletItems(value) {
  const items = [];
  for (const line of value.split('\n')) {
    if (line.startsWith('- ')) items.push(line.slice(2));
    else if (/^\s+\S/.test(line) && items.length) items[items.length - 1] += ` ${line.trim()}`;
  }
  return items;
}

function renderMarkdown(value) {
  const lines = value.split('\n');
  const blocks = [];
  for (let index = 0; index < lines.length;) {
    const line = lines[index];
    if (!line.trim()) { index++; continue; }
    const heading = /^(#{1,3}) (.+)$/.exec(line);
    if (heading) {
      const id = heading[2].toLowerCase().replace(/[^a-z0-9 ]/g, '').replace(/ +/g, '-');
      blocks.push(`<h${heading[1].length} id="${id}">${inline(heading[2])}</h${heading[1].length}>`);
      index++;
    } else if (line.startsWith('|')) {
      const rows = [];
      while (index < lines.length && lines[index].startsWith('|')) {
        rows.push(lines[index++].split(/(?<!\\)\|/).slice(1, -1).map((cell) => cell.trim()));
      }
      const headers = rows.shift();
      if (!rows.shift()?.every((cell) => /^:?-+:?$/.test(cell))) throw new Error('Malformed status table');
      blocks.push(`<div class="table-scroll" tabindex="0" role="region" aria-label="${escape(headers.join(', '))}"><table><thead><tr>${headers.map((cell) => `<th scope="col">${inline(cell)}</th>`).join('')}</tr></thead><tbody>${rows.map((row) => `<tr>${row.map((cell) => `<td>${inline(cell)}</td>`).join('')}</tr>`).join('')}</tbody></table></div>`);
    } else if (line.startsWith('- ')) {
      const list = [];
      while (index < lines.length && (lines[index].startsWith('- ') || /^\s+\S/.test(lines[index]))) list.push(lines[index++]);
      blocks.push(`<ul>${bulletItems(list.join('\n')).map((item) => `<li>${inline(item)}</li>`).join('')}</ul>`);
    } else {
      const paragraph = [];
      while (index < lines.length && lines[index].trim() && !/^(#|\||- )/.test(lines[index])) paragraph.push(lines[index++].trim());
      if (!paragraph.length) throw new Error(`Unsupported Markdown at line ${index + 1}`);
      blocks.push(`<p>${inline(paragraph.join(' '))}</p>`);
    }
  }
  return blocks.join('\n');
}

const normalizedStatus = status.replace(/\r\n/g, '\n');
// Normalize once before the section parser so Windows and Linux generate the same site.
const getSection = (title) => {
  const marker = `## ${title}\n`;
  const start = normalizedStatus.indexOf(marker);
  if (start < 0) throw new Error(`FORK_STATUS.md is missing: ${title}`);
  return normalizedStatus.slice(start + marker.length).split('\n## ')[0].trim();
};
const rows = getSection('Implemented changes').split('\n').filter((line) => line.startsWith('|')).slice(2)
  .map((line) => line.split(/(?<!\\)\|/).slice(1, -1).map((cell) => cell.trim()));
const changes = new Map(rows.map(([area, description]) => [area, description]));
// Editorial summaries explain user impact; technical evidence stays in FORK_STATUS.md.
const highlights = [
  ['01', 'Tabs & WSL', 'A new tab, in the right place.', 'WSL working directories', 'When directory inheritance is enabled, new tabs open in the focused tab’s working directory. WSL zsh integration reports that location automatically.'],
  ['02', 'Text & languages', 'Give every tab a readable title.', 'Tab title rendering', 'Better shaping, font fallback, and alignment for mixed-language titles. Set a dedicated tab font and size without changing the fonts inside your terminal.'],
  ['03', 'GPU memory', 'Reserve less. Grow when needed.', 'Grayscale glyph atlases', 'Glyph storage starts smaller and expands as more characters are used. Color glyphs grow on demand too, keeping existing characters intact.'],
  ['04', 'Windows displays', 'Keep presentation moving.', 'Windows VRR presentation', 'Focused, visible maximized and fullscreen windows keep presenting, even when text is idle. This trades some idle GPU work for steady presentation; other window states remain event-driven.'],
];
const changesHtml = highlights.map(([number, label, title, area, description]) => {
  if (!changes.has(area)) throw new Error(`Missing highlighted change: ${area}`);
  return `<article class="change-row"><div class="change-index">${number}<span>${label}</span></div><h3>${title}</h3><p>${inline(description)}</p></article>`;
}).join('\n');
const releaseItems = bulletItems(getSection('Release and compatibility status'));
if (releaseItems.length < 3) throw new Error('Expected release and platform status bullets');
const reviewDate = /Last reviewed: \*\*(\d{4}-\d{2}-\d{2})\*\*/.exec(normalizedStatus)?.[1];
if (!reviewDate) throw new Error('FORK_STATUS.md is missing its review date');
const reviewLabel = new Intl.DateTimeFormat('en-GB', { day: 'numeric', month: 'short', year: 'numeric', timeZone: 'UTC' }).format(new Date(`${reviewDate}T00:00:00Z`));
const rustVersion = /rust-version\s*=\s*"([^"]+)"/.exec(cargo)?.[1];
if (!rustVersion) throw new Error('Cargo.toml is missing rust-version');
const replacements = {
  SCREENSHOT_URL: escape(`${base}tacterm-windows.png?v=${screenshotVersion}`),
  SCREENSHOT_WIDTH: screenshotWidth, SCREENSHOT_HEIGHT: screenshotHeight,
  THEME_SWITCHER: `<div class="theme-switcher" role="group" aria-label="Color theme" hidden><button type="button" data-theme-choice="light" aria-pressed="false" aria-label="Light theme" title="Light theme"><span aria-hidden="true">☀</span><span class="theme-label">Light</span></button><button type="button" data-theme-choice="dark" aria-pressed="false" aria-label="Dark theme" title="Dark theme"><span aria-hidden="true">☾</span><span class="theme-label">Dark</span></button><button type="button" data-theme-choice="system" aria-pressed="true" aria-label="System theme" title="Follow system theme"><span aria-hidden="true">◐</span><span class="theme-label">Auto</span></button></div>`,
  BASE: escape(base), URL: escape(siteUrl.href), COUNT: rows.length,
  REVIEW_DATE: escape(reviewLabel), RUST_VERSION: escape(rustVersion),
  CHANGES: changesHtml,
  HERO_NOTE: release ? `${release.prerelease ? 'Preview' : 'Release'} ${escape(release.version)} · Windows, macOS &amp; Linux` : 'Windows, macOS &amp; Linux · Build from source or visit GitHub Releases',
  COMMIT: escape(process.env.GITHUB_SHA || 'local-preview'),
  DOWNLOADS: downloadSection(),
  HERO_ACTIONS: release
    ? `<a class="button button-dark" href="#downloads">${release.prerelease ? 'Download preview' : 'Download tacterm'} <span aria-hidden="true">↓</span></a><a class="text-link" href="#changes">Explore the changes <span aria-hidden="true">↗</span></a>`
    : '<a class="button button-dark" href="#changes">Explore the changes <span aria-hidden="true">↗</span></a><a class="text-link" href="#build">Build from source <span aria-hidden="true">↓</span></a>',
};
function fill(template, extra = {}) {
  const values = { ...replacements, ...extra };
  return template.replace(/\{\{([A-Z_]+)\}\}/g, (_, key) => {
    if (!(key in values)) throw new Error(`Unknown template value: ${key}`);
    return String(values[key]);
  });
}
const detailShell = (title, content, path = 'status.html') => fill(`<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>${escape(title)} · tacterm</title><meta name="description" content="Implemented changes, validation, and release status for tacterm, a terminal based on Rio."><meta name="source-commit" content="{{COMMIT}}"><link rel="canonical" href="{{URL}}${path}"><link rel="icon" href="{{BASE}}favicon.svg" type="image/svg+xml"><meta name="theme-color" content="#f5f4ee"><script src="{{BASE}}theme.js"></script><link rel="stylesheet" href="{{BASE}}styles.css"></head><body><a class="skip-link" href="#main">Skip to content</a><header class="site-header wrap"><a class="wordmark" href="{{BASE}}" aria-label="tacterm home"><span class="brand-mark" aria-hidden="true">t_</span>tacterm</a><nav aria-label="Main navigation"><a href="{{BASE}}#changes">What's different</a><a href="https://github.com/narma/tacterm">GitHub ↗</a></nav>{{THEME_SWITCHER}}</header><main id="main" class="wrap status-document"><a class="back-link" href="{{BASE}}">← Back to tacterm</a>${content}</main><footer class="site-footer wrap"><span>tacterm / based on Rio</span><a href="https://github.com/narma/tacterm/blob/main/FORK_STATUS.md">Read the source document ↗</a></footer></body></html>`);

await mkdir(outDir, { recursive: true });
await writeFile(join(outDir, 'index.html'), fill(await readFile(join(sourceDir, 'index.html'), 'utf8')));
await writeFile(join(outDir, 'status.html'), detailShell('Project status', renderMarkdown(normalizedStatus).replace('</h1>', '</h1><nav class="status-toc" aria-label="On this page"><a href="#implemented-changes">Changes</a><a href="#performance-and-limits">Performance &amp; limits</a><a href="#validation">Validation</a><a href="#release-and-compatibility-status">Release &amp; compatibility</a></nav>')));
await writeFile(join(outDir, '404.html'), detailShell('Page not found', '<p class="eyebrow">404 / PAGE NOT FOUND</p><h1>Let’s get you<br>back to work.</h1><p>This page has moved or does not exist.</p><a class="button button-dark" href="{{BASE}}">Go to tacterm <span aria-hidden="true">↗</span></a>', '404.html'));
for (const asset of ['styles.css', 'site.js', 'theme.js', 'favicon.svg', 'tacterm-windows.png']) await copyFile(join(sourceDir, asset), join(outDir, asset));
await writeFile(join(outDir, '.nojekyll'), '');
await writeFile(join(outDir, 'robots.txt'), `User-agent: *\nAllow: /\nSitemap: ${siteUrl.href}sitemap.xml\n`);
await writeFile(join(outDir, 'sitemap.xml'), `<?xml version="1.0" encoding="UTF-8"?><urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9"><url><loc>${escape(siteUrl.href)}</loc></url><url><loc>${escape(siteUrl.href)}status.html</loc></url></urlset>\n`);
console.log(`Built tacterm: ${outDir}\nURL: ${siteUrl.href}\nStatus: ${rows.length} changes, reviewed ${reviewDate}\nDownloads: ${release?.version || 'release metadata not loaded'}`);
