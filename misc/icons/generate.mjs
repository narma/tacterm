// Development tool only; application builds use the committed icon files.
import { createRequire } from 'node:module';
import { readFile, writeFile } from 'node:fs/promises';
import { resolve, dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const args = process.argv.slice(2);
const toolsIndex = args.indexOf('--tools');
const tools = resolve(toolsIndex < 0 ? 'target/icon-tools' : args[toolsIndex + 1]);
const require = createRequire(join(tools, 'package.json'));
const sharp = require('sharp');
if (sharp.versions.sharp !== '0.35.5') throw new Error('Use sharp 0.35.5 to regenerate icons');
const directory = dirname(fileURLToPath(import.meta.url));
const svg = await readFile(join(directory, 'tacterm.svg'));
const sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256, 512, 1024];
const png = new Map(await Promise.all(sizes.map(async (size) => [size, await sharp(svg, { density: Math.max(72, size / 64 * 72) }).resize(size, size).png().toBuffer()])));

// Windows ICO: separate PNG-compressed frames for small/large icons and DPI scales.
const icoSizes = sizes.filter((size) => size <= 256);
const header = Buffer.alloc(6 + icoSizes.length * 16);
header.writeUInt16LE(1, 2);
header.writeUInt16LE(icoSizes.length, 4);
let offset = header.length;
for (const [index, size] of icoSizes.entries()) {
  const position = 6 + index * 16;
  header[position] = header[position + 1] = size === 256 ? 0 : size;
  header.writeUInt16LE(1, position + 4);
  header.writeUInt16LE(32, position + 6);
  header.writeUInt32LE(png.get(size).length, position + 8);
  header.writeUInt32LE(offset, position + 12);
  offset += png.get(size).length;
}
const ico = Buffer.concat([header, ...icoSizes.map((size) => png.get(size))]);

// Modern ICNS PNG representations, including explicit Retina slots.
const representations = [['icp4', 16], ['icp5', 32], ['icp6', 64], ['ic07', 128], ['ic08', 256], ['ic09', 512], ['ic10', 1024], ['ic11', 32], ['ic12', 64], ['ic13', 256], ['ic14', 512]];
const chunks = representations.map(([type, size]) => {
  const chunk = Buffer.alloc(8);
  chunk.write(type, 0, 'ascii');
  chunk.writeUInt32BE(png.get(size).length + 8, 4);
  return Buffer.concat([chunk, png.get(size)]);
});
const icnsHeader = Buffer.alloc(8);
icnsHeader.write('icns', 0, 'ascii');
icnsHeader.writeUInt32BE(8 + chunks.reduce((length, chunk) => length + chunk.length, 0), 4);
const outputs = new Map([['tacterm-256.png', png.get(256)], ['tacterm-1024.png', png.get(1024)], ['tacterm.ico', ico], ['tacterm.icns', Buffer.concat([icnsHeader, ...chunks])]]);
for (const [name, bytes] of outputs) {
  const path = join(directory, name);
  if (args.includes('--check')) {
    if (!(await readFile(path)).equals(bytes)) throw new Error(`Icon is stale: ${name}`);
  } else await writeFile(path, bytes);
  console.log(`${args.includes('--check') ? 'Checked' : 'Generated'} ${name}: ${bytes.length} bytes`);
}
