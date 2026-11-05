/**
 * Copies the `lm-lathe-server` binary into the directory Tauri expects for
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
  join(repoRoot, "target", "release", "lm-lathe-server"),
  join(repoRoot, "target", "debug", "lm-lathe-server"),
];

// The backend is always built first: an incremental build takes a second when
// nothing changed and prevents the application from starting a stale copy.
console.log("[sidecar] building lm-lathe-server …");
execFileSync("cargo", ["build", "--release", "-p", "lm-lathe-server"], {
  cwd: repoRoot,
  stdio: "inherit",
});

let source = candidates.find((path) => existsSync(path));
if (!source) {
  console.error("[sidecar] the backend was not built");
  process.exit(1);
}

mkdirSync(binaries, { recursive: true });
const destination = join(binaries, `lm-lathe-server-${triple}`);
copyFileSync(source, destination);
console.log(`[sidecar] ${source} -> ${destination}`);
