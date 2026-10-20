import test from 'node:test';
import assert from 'node:assert/strict';
import { validateRelease } from './releases.mjs';

function completeRelease() {
  const version = '0.1.0-alpha.1';
  const tag = `tacterm-v${version}`;
  return {
    schemaVersion: 1, version, tag, prerelease: true,
    assets: ['windows', 'macos', 'linux'].flatMap((platform) => ['x64', 'arm64'].map((arch) => {
      const name = `tacterm-${version}-${platform}-${arch}.${platform === 'linux' ? 'tar.gz' : 'zip'}`;
      return { platform, arch, name, sha256: 'a'.repeat(64), bytes: 100, url: `https://github.com/narma/tacterm/releases/download/${tag}/${name}` };
    })),
  };
}

test('offers every platform and architecture of a complete published preview', () => {
  const result = validateRelease(completeRelease());
  assert.equal(result.assets.length, 6);
  assert.equal(result.prerelease, true);
  assert.match(result.checksumUrl, /\/tacterm-v0\.1\.0-alpha\.1\/SHA256SUMS$/);
});

function installerRelease() {
  const manifest = completeRelease();
  manifest.schemaVersion = 2;
  manifest.assets = manifest.assets.flatMap((asset) => {
    if (asset.platform !== 'windows') return [{ ...asset, kind: 'archive' }];
    return ['portable', 'setup'].map((kind) => {
      const name = `tacterm-${manifest.version}-windows-${asset.arch}-${kind}.${kind === 'setup' ? 'exe' : 'zip'}`;
      return { ...asset, kind, name, url: `https://github.com/narma/tacterm/releases/download/${manifest.tag}/${name}` };
    });
  });
  return manifest;
}

test('accepts portable and setup for both Windows architectures', () => {
  assert.equal(validateRelease(installerRelease()).assets.length, 8);
});

test('rejects missing or duplicate installers', () => {
  const missing = installerRelease();
  missing.assets.pop();
  assert.throws(() => validateRelease(missing), /both Windows installers/);
  const duplicate = installerRelease();
  duplicate.assets[1] = duplicate.assets[0];
  assert.throws(() => validateRelease(duplicate), /duplicate/);
});

test('rejects a setup download disguised as a ZIP', () => {
  const manifest = installerRelease();
  manifest.assets[1].name = manifest.assets[1].name.replace('.exe', '.zip');
  assert.throws(() => validateRelease(manifest), /metadata/);
});

test('does not offer a partial matrix as a completed release', () => {
  const manifest = completeRelease();
  manifest.assets.pop();
  assert.throws(() => validateRelease(manifest), /all six/);
});

test('rejects duplicate architectures even when there are six attachments', () => {
  const manifest = completeRelease();
  manifest.assets[1] = manifest.assets[0];
  assert.throws(() => validateRelease(manifest), /duplicate/);
});

test('rejects download URLs outside this repository release', () => {
  const manifest = completeRelease();
  manifest.assets[0].url = 'https://example.com/another-binary.zip';
  assert.throws(() => validateRelease(manifest), /download host/);
});

test('rejects version/tag mismatches and missing checksums', () => {
  const mismatch = completeRelease();
  mismatch.tag = 'tacterm-v9.0.0';
  assert.throws(() => validateRelease(mismatch), /manifest/);
  const missing = completeRelease();
  missing.assets[0].sha256 = '';
  assert.throws(() => validateRelease(missing), /metadata/);
});
