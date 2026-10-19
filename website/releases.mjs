import { readFile } from 'node:fs/promises';

const repo = 'narma/tacterm';
const allowedPlatforms = ['windows', 'macos', 'linux'];
export function validateRelease(manifest) {
  if (manifest.schemaVersion !== 1 || !/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(manifest.version) || manifest.tag !== `tacterm-v${manifest.version}`) throw new Error('Unsupported tacterm release manifest');
  if (!Array.isArray(manifest.assets) || manifest.assets.length !== 6) throw new Error('Expected all six native downloads');
  const ids = new Set();
  for (const asset of manifest.assets) {
    const id = `${asset.platform}-${asset.arch}`;
    if (!allowedPlatforms.includes(asset.platform) || !['x64', 'arm64'].includes(asset.arch) || ids.has(id)) throw new Error('Invalid or duplicate download architecture');
    ids.add(id);
    const extension = asset.platform === 'linux' ? 'tar.gz' : 'zip';
    if (asset.name !== `tacterm-${manifest.version}-${id}.${extension}` || !/^[a-f0-9]{64}$/.test(asset.sha256) || !(asset.bytes > 0)) throw new Error('Invalid download metadata');
    const expected = `https://github.com/${repo}/releases/download/${manifest.tag}/${asset.name}`;
    if (asset.url !== expected) throw new Error('Unexpected download host or path');
  }
  return { ...manifest, releaseUrl: `https://github.com/${repo}/releases/tag/${manifest.tag}`, checksumUrl: `https://github.com/${repo}/releases/download/${manifest.tag}/SHA256SUMS` };
}

export async function loadRelease({ file, online = false } = {}) {
  if (file) return validateRelease(JSON.parse(await readFile(file, 'utf8')));
  if (!online) return null;
  const headers = { Accept: 'application/vnd.github+json', 'X-GitHub-Api-Version': '2022-11-28' };
  if (process.env.GH_TOKEN) headers.Authorization = `Bearer ${process.env.GH_TOKEN}`;
  const response = await fetch(`https://api.github.com/repos/${repo}/releases?per_page=50`, { headers, signal: AbortSignal.timeout(30000) });
  if (!response.ok) throw new Error(`Cannot read published tacterm releases: HTTP ${response.status}`);
  const releases = (await response.json()).filter((release) => !release.draft && release.tag_name.startsWith('tacterm-v')).sort((a, b) => b.published_at.localeCompare(a.published_at));
  if (!releases.length) return null;
  const release = releases[0];
  const manifestAsset = release.assets.find((asset) => asset.name === 'release.json');
  const checksumAsset = release.assets.find((asset) => asset.name === 'SHA256SUMS');
  if (!manifestAsset || !checksumAsset) throw new Error('Published release is missing its manifest or checksums');
  const result = await fetch(manifestAsset.browser_download_url, { signal: AbortSignal.timeout(30000) });
  if (!result.ok) throw new Error(`Cannot read release manifest: HTTP ${result.status}`);
  const manifest = validateRelease(await result.json());
  if (manifest.tag !== release.tag_name) throw new Error('Release tag and manifest disagree');
  for (const download of manifest.assets) {
    const asset = release.assets.find((item) => item.name === download.name);
    if (!asset || asset.size !== download.bytes || asset.browser_download_url !== download.url) throw new Error(`Published download is incomplete: ${download.name}`);
    if (asset.digest && asset.digest !== `sha256:${download.sha256}`) throw new Error(`Published download checksum mismatch: ${download.name}`);
  }
  return { ...manifest, prerelease: release.prerelease };
}
