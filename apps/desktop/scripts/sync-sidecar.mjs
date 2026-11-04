/**
 * Copies the `tts-server` binary into the directory Tauri expects for
 * sidecars. Tauri names the file after the target triple.
 */
import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(here, "..", "..", "..");
const binaries = join(here, "..", "src-tauri", "binaries");

const arch = { arm64: "aarch64", x64: "x86_64" }[process.arch] ?? process.arch;
const platform = {
  darwin: "apple-darwin",
  linux: "unknown-linux-gnu",
  win32: "windows-msvc",
}[process.platform] ?? process.platform;
const triple = process.env.TAURI_ENV_TARGET_TRIPLE ?? `${arch}-${platform}`;

const candidates = [
  join(repoRoot, "target", "release", "tts-server"),
  join(repoRoot, "target", "debug", "tts-server"),
];

let source = candidates.find((path) => existsSync(path));
if (!source) {
  console.log("[sidecar] building tts-server …");
  execFileSync("cargo", ["build", "--release", "-p", "tts-server"], {
    cwd: repoRoot,
    stdio: "inherit",
  });
  source = candidates[0];
}

mkdirSync(binaries, { recursive: true });
const destination = join(binaries, `tts-server-${triple}`);
copyFileSync(source, destination);
console.log(`[sidecar] ${source} -> ${destination}`);
