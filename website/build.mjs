import { readFile, writeFile, mkdir, copyFile } from 'node:fs/promises';
import { resolve, dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
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

function downloadSection() {
  if (!release) return `<section id="downloads" class="downloads-section wrap" aria-labelledby="downloads-title"><p class="eyebrow">RELEASE BUILDS</p><h2 id="downloads-title">The first preview<br>is on its way.</h2><p class="download-intro">Ready-made downloads appear here after every native build and release check passes. In the meantime, you can <a href="#build">build from source</a>.</p><a class="text-link" href="${repo}/releases">Follow GitHub Releases ↗</a></section>`;
  const labels = { windows: 'Windows', macos: 'macOS', linux: 'Linux' };
  const notes = { windows: 'Portable ZIP. Unsigned binaries; Windows may show an unknown-publisher warning.', macos: 'Tacterm.app · macOS 14+. Ad-hoc signed; not notarized. Gatekeeper may block the first launch.', linux: 'tar.gz · glibc 2.35+ (Ubuntu 22.04+). Requires font libraries and a Vulkan-capable driver.' };
  const cards = Object.entries(labels).map(([platform, label]) => {
    const preferredArch = platform === 'macos' ? 'arm64' : 'x64';
    const links = release.assets.filter((asset) => asset.platform === platform).sort((a, b) => Number(b.arch === preferredArch) - Number(a.arch === preferredArch)).map((asset) => {
      const arch = platform === 'macos' ? (asset.arch === 'arm64' ? 'Apple Silicon' : 'Intel') : (asset.arch === 'x64' ? 'x64' : 'ARM64');
      return `<a class="download-link" href="${escape(asset.url)}"><span>${arch}</span><span>${(asset.bytes / 1048576).toFixed(1)} MB &nbsp; ↓</span></a>`;
    }).join('');
    return `<article class="download-card"><h3>${label}</h3>${links}<p>${notes[platform]}</p></article>`;
  }).join('');
  return `<section id="downloads" class="downloads-section wrap" aria-labelledby="downloads-title"><div class="section-heading"><div><p class="eyebrow">${release.prerelease ? 'PREVIEW RELEASE' : 'RELEASE BUILDS'} / ${escape(release.version)}</p><h2 id="downloads-title">Ready when<br>you are.</h2></div><a class="text-link" href="${escape(release.releaseUrl)}">Release notes &amp; installation ↗</a></div><p class="download-intro">Choose your system and processor. These early builds share Rio's settings paths; every archive includes installation notes.</p><div class="download-grid">${cards}</div><div class="download-footer"><a class="text-link" href="${escape(release.checksumUrl)}">SHA256 checksums ↗</a><a class="text-link" href="${escape(release.releaseUrl)}">All assets on GitHub ↗</a><span>Native builds and CLI smoke checks. GUI/GPU validation is separate.</span></div></section>`;
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
const highlights = [
  ['01', 'Resources', 'Room to grow. Less to reserve.', 'Grayscale glyph atlases'],
  ['02', 'Presentation', 'A steadier window into your work.', 'Windows VRR presentation'],
  ['03', 'Typography', 'Every title deserves a little attention.', 'Tab title rendering'],
  ['04', 'Workflow', 'A new tab. The same working directory.', 'WSL working directories'],
];
const changesHtml = highlights.map(([number, label, title, area]) => {
  if (!changes.has(area)) throw new Error(`Missing highlighted change: ${area}`);
  return `<article class="change-row"><div class="change-index">${number}<span>${label}</span></div><h3>${title}</h3><p>${inline(changes.get(area))}</p></article>`;
}).join('\n');
const releaseItems = bulletItems(getSection('Release and compatibility status'));
if (releaseItems.length < 3) throw new Error('Expected release and platform status bullets');
const reviewDate = /Last reviewed: \*\*(\d{4}-\d{2}-\d{2})\*\*/.exec(normalizedStatus)?.[1];
if (!reviewDate) throw new Error('FORK_STATUS.md is missing its review date');
const reviewLabel = new Intl.DateTimeFormat('en-GB', { day: 'numeric', month: 'short', year: 'numeric', timeZone: 'UTC' }).format(new Date(`${reviewDate}T00:00:00Z`));
const rustVersion = /rust-version\s*=\s*"([^"]+)"/.exec(cargo)?.[1];
if (!rustVersion) throw new Error('Cargo.toml is missing rust-version');
const replacements = {
  BASE: escape(base), URL: escape(siteUrl.href), COUNT: rows.length,
  REVIEW_DATE: escape(reviewLabel), RUST_VERSION: escape(rustVersion),
  CHANGES: changesHtml,
  STATUS_SUMMARY: releaseItems.slice(0, 3).map((item) => `<li>${inline(item)}</li>`).join('\n'),
  COMMIT: escape(process.env.GITHUB_SHA || 'local-preview'),
  DOWNLOADS: downloadSection(),
  HERO_ACTIONS: release
    ? '<a class="button button-dark" href="#downloads">Download preview <span aria-hidden="true">↓</span></a><a class="text-link" href="#changes">Explore the changes <span aria-hidden="true">↗</span></a>'
    : '<a class="button button-dark" href="#changes">Explore the changes <span aria-hidden="true">↗</span></a><a class="text-link" href="#build">Build from source <span aria-hidden="true">↓</span></a>',
  BUILD_INTRO: release ? 'Prefer to build it yourself? The complete source is available. Build a development version' : 'Public tacterm packages are still in preparation. You can build a development version',
};
function fill(template, extra = {}) {
  const values = { ...replacements, ...extra };
  return template.replace(/\{\{([A-Z_]+)\}\}/g, (_, key) => {
    if (!(key in values)) throw new Error(`Unknown template value: ${key}`);
    return String(values[key]);
  });
}
const detailShell = (title, content, path = 'status.html') => fill(`<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>${escape(title)} · tacterm</title><meta name="description" content="Implemented changes, validation, and release status for tacterm, a terminal based on Rio."><meta name="source-commit" content="{{COMMIT}}"><link rel="canonical" href="{{URL}}${path}"><link rel="icon" href="{{BASE}}favicon.svg" type="image/svg+xml"><link rel="stylesheet" href="{{BASE}}styles.css"></head><body><a class="skip-link" href="#main">Skip to content</a><header class="site-header wrap"><a class="wordmark" href="{{BASE}}" aria-label="tacterm home"><span class="brand-mark" aria-hidden="true">t_</span>tacterm</a><nav aria-label="Main navigation"><a href="{{BASE}}#changes">What's different</a><a href="https://github.com/narma/tacterm">GitHub ↗</a></nav></header><main id="main" class="wrap status-document"><a class="back-link" href="{{BASE}}">← Back to tacterm</a>${content}</main><footer class="site-footer wrap"><span>tacterm / based on Rio</span><a href="https://github.com/narma/tacterm/blob/main/FORK_STATUS.md">Read the source document ↗</a></footer></body></html>`);

await mkdir(outDir, { recursive: true });
await writeFile(join(outDir, 'index.html'), fill(await readFile(join(sourceDir, 'index.html'), 'utf8')));
await writeFile(join(outDir, 'status.html'), detailShell('Fork status', renderMarkdown(normalizedStatus)));
await writeFile(join(outDir, '404.html'), detailShell('Page not found', '<p class="eyebrow">404 / PAGE NOT FOUND</p><h1>Let’s get you<br>back to work.</h1><p>This page has moved or does not exist.</p><a class="button button-dark" href="{{BASE}}">Go to tacterm <span aria-hidden="true">↗</span></a>', '404.html'));
for (const asset of ['styles.css', 'site.js', 'favicon.svg']) await copyFile(join(sourceDir, asset), join(outDir, asset));
await writeFile(join(outDir, '.nojekyll'), '');
await writeFile(join(outDir, 'robots.txt'), `User-agent: *\nAllow: /\nSitemap: ${siteUrl.href}sitemap.xml\n`);
await writeFile(join(outDir, 'sitemap.xml'), `<?xml version="1.0" encoding="UTF-8"?><urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9"><url><loc>${escape(siteUrl.href)}</loc></url><url><loc>${escape(siteUrl.href)}status.html</loc></url></urlset>\n`);
console.log(`Built tacterm: ${outDir}\nURL: ${siteUrl.href}\nStatus: ${rows.length} changes, reviewed ${reviewDate}\nDownloads: ${release?.version || 'no published release yet'}`);
