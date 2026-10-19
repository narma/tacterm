import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, copyFile, readFile, writeFile } from 'node:fs/promises';
import { join, resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const version = (await readFile(join(root, 'packaging/VERSION'), 'utf8')).trim();
const sha = '1234567890abcdef1234567890abcdef12345678';
async function fixture() {
  const parent = join(root, 'target/release-tests');
  await mkdir(parent, { recursive: true });
  const directory = await mkdtemp(join(parent, 'case-'));
  await mkdir(join(directory, 'packaging'));
  for (const name of ['release.mjs', 'targets.json', 'VERSION']) await copyFile(join(root, 'packaging', name), join(directory, 'packaging', name));
  await copyFile(join(root, 'Cargo.toml'), join(directory, 'Cargo.toml'));
  const assets = join(directory, 'target/release-assets');
  await mkdir(assets, { recursive: true });
  const targets = JSON.parse(await readFile(join(directory, 'packaging/targets.json'), 'utf8'));
  for (const target of targets) {
    const archive = `tacterm-${version}-${target.id}.${target.format}`;
    const bytes = Buffer.from(`fixture archive for ${target.id}`);
    await writeFile(join(assets, archive), bytes);
    await writeFile(join(assets, `build-${target.id}.json`), JSON.stringify({ ...target, version, commit: sha, archive, archiveSha256: createHash('sha256').update(bytes).digest('hex'), bytes: bytes.length }));
  }
  const run = (command, env = {}) => spawnSync(process.execPath, [join(directory, 'packaging/release.mjs'), command], { encoding: 'utf8', env: { ...process.env, GITHUB_SHA: sha, GITHUB_OUTPUT: '', ...env } });
  return { directory, assets, targets, run };
}

test('assembles the complete native matrix into a checksum-bearing public manifest', async () => {
  const f = await fixture();
  const result = f.run('assemble');
  assert.equal(result.status, 0, result.stderr);
  const manifest = JSON.parse(await readFile(join(f.assets, 'release.json'), 'utf8'));
  assert.equal(manifest.assets.length, 6);
  assert.equal(manifest.commit, sha);
  const checksums = await readFile(join(f.assets, 'SHA256SUMS'), 'utf8');
  assert.equal(checksums.trim().split('\n').length, 7);
  for (const asset of manifest.assets) assert.equal(asset.url, `https://github.com/narma/tacterm/releases/download/tacterm-v${version}/${asset.name}`);
});

test('rejects archives corrupted after packaging', async () => {
  const f = await fixture();
  await writeFile(join(f.assets, `tacterm-${version}-windows-x64.zip`), 'modified');
  const result = f.run('assemble');
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /checksum mismatch/);
});

test('rejects a matrix artifact from a different source commit', async () => {
  const f = await fixture();
  const file = join(f.assets, 'build-linux-x64.json');
  const record = JSON.parse(await readFile(file, 'utf8'));
  record.commit = 'a'.repeat(40);
  await writeFile(file, JSON.stringify(record));
  const result = f.run('assemble');
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /same release commit and version/);
});

test('rejects a tag that differs from the version committed in source', async () => {
  const f = await fixture();
  const result = f.run('prepare', { GITHUB_REF_TYPE: 'tag', GITHUB_REF_NAME: 'tacterm-v99.0.0' });
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /Tag must match committed version/);
});
