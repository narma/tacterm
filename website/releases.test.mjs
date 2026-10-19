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
