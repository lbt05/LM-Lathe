/**
 * Draws the application icon.
 *
 * The icon is a dark rounded square with a voice waveform. It is written as
 * a PNG without any image library, so the build has no extra dependency.
 */
import { deflateSync } from "node:zlib";
import { writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const size = 1024;
const here = dirname(fileURLToPath(import.meta.url));

const background = [15, 17, 21, 255];
const accent = [91, 140, 255, 255];
const accentSoft = [91, 140, 255, 90];

const pixels = Buffer.alloc(size * size * 4);
for (let y = 0; y < size; y += 1) {
  for (let x = 0; x < size; x += 1) {
    const offset = (y * size + x) * 4;
    pixels[offset] = background[0];
    pixels[offset + 1] = background[1];
    pixels[offset + 2] = background[2];
    pixels[offset + 3] = background[3];
  }
}

/** Draws a rounded rectangle. */
function roundedRect(x0, y0, width, height, radius, colour) {
  for (let y = y0; y < y0 + height; y += 1) {
    for (let x = x0; x < x0 + width; x += 1) {
      const dx = Math.max(x0 + radius - x, x - (x0 + width - 1 - radius), 0);
      const dy = Math.max(y0 + radius - y, y - (y0 + height - 1 - radius), 0);
      if (dx * dx + dy * dy > radius * radius) continue;
      const offset = (y * size + x) * 4;
      pixels[offset] = colour[0];
      pixels[offset + 1] = colour[1];
      pixels[offset + 2] = colour[2];
      pixels[offset + 3] = colour[3];
    }
  }
}

// A soft background plate.
roundedRect(120, 120, 784, 784, 190, accentSoft);

// The waveform: five rounded bars.
const bars = [0.34, 0.66, 1.0, 0.72, 0.42];
const barWidth = 92;
const gap = 44;
const totalWidth = bars.length * barWidth + (bars.length - 1) * gap;
let cursor = Math.round((size - totalWidth) / 2);
for (const factor of bars) {
  const height = Math.round(470 * factor);
  const y = Math.round((size - height) / 2);
  roundedRect(cursor, y, barWidth, height, barWidth / 2, accent);
  cursor += barWidth + gap;
}

/** Encodes the pixel buffer as a PNG. */
function encodePng() {
  const raw = Buffer.alloc((size * 4 + 1) * size);
  for (let y = 0; y < size; y += 1) {
    raw[y * (size * 4 + 1)] = 0; // filter: none
    pixels.copy(raw, y * (size * 4 + 1) + 1, y * size * 4, (y + 1) * size * 4);
  }

  const chunk = (type, data) => {
    const length = Buffer.alloc(4);
    length.writeUInt32BE(data.length);
    const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
    const crc = Buffer.alloc(4);
    crc.writeUInt32BE(crc32(body) >>> 0);
    return Buffer.concat([length, body, crc]);
  };

  const header = Buffer.alloc(13);
  header.writeUInt32BE(size, 0);
  header.writeUInt32BE(size, 4);
  header[8] = 8; // bit depth
  header[9] = 6; // colour type: RGBA
  header[10] = 0;
  header[11] = 0;
  header[12] = 0;

  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", header),
    chunk("IDAT", deflateSync(raw, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

const table = (() => {
  const values = new Int32Array(256);
  for (let n = 0; n < 256; n += 1) {
    let c = n;
    for (let k = 0; k < 8; k += 1) {
      c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    }
    values[n] = c;
  }
  return values;
})();

function crc32(buffer) {
  let crc = 0xffffffff;
  for (const byte of buffer) {
    crc = table[(crc ^ byte) & 0xff] ^ (crc >>> 8);
  }
  return crc ^ 0xffffffff;
}

const target = join(here, "..", "src-tauri", "icons", "icon.png");
writeFileSync(target, encodePng());
console.log(`[icon] ${target}`);
