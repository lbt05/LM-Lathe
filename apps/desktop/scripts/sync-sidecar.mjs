/**
 * Copies the `lm-lathe-server` binary into the directory Tauri expects for
 * sidecars and fetches the tools that travel with the application.
 *
 * Tauri names the sidecar files after the target triple.
 */
import { execFileSync } from "node:child_process";
import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  rmSync,
} from "node:fs";
import { tmpdir } from "node:os";
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

/**
 * The version of `uv` that travels with the application.
 *
 * `uv` installs the Python packages of the engines. It is a single static
 * binary, so it can be bundled: the user does not have to install anything
 * before an engine can be installed.
 */
const UV_VERSION = process.env.LM_LATHE_UV_VERSION ?? "0.13.7";

/**
 * Fetches `uv` for the target platform.
 *
 * The binary is kept in the sidecar directory: the next build does not
 * download it again, so the application can be built without a network.
 */
function fetchUv() {
  const target = join(binaries, `uv-${triple}`);
  if (existsSync(target)) {
    console.log(`[sidecar] uv ${UV_VERSION} is already there`);
    return;
  }

  // Windows publishes a zip archive, the other platforms a tarball.
  if (process.platform === "win32") {
    console.warn("[sidecar] uv is not fetched for Windows yet");
    console.warn("[sidecar] install uv with `winget install astral-sh.uv`");
    return;
  }

  const archive = `uv-${triple}.tar.gz`;
  const url = `https://github.com/astral-sh/uv/releases/download/${UV_VERSION}/${archive}`;
  const scratch = mkdtempSync(join(tmpdir(), "lm-lathe-uv-"));
  try {
    console.log(`[sidecar] fetching uv ${UV_VERSION} …`);
    execFileSync("curl", ["-sSfL", "-o", join(scratch, archive), url], {
      stdio: "inherit",
    });
    execFileSync("tar", ["-xzf", join(scratch, archive), "-C", scratch], {
      stdio: "inherit",
    });
    // The archive holds one directory that is named after the platform.
    const unpacked = readdirSync(scratch).find((entry) =>
      existsSync(join(scratch, entry, "uv")),
    );
    if (!unpacked) {
      throw new Error(`the archive ${archive} does not hold a uv binary`);
    }
    copyFileSync(join(scratch, unpacked, "uv"), target);
    chmodSync(target, 0o755);
    console.log(`[sidecar] uv -> ${target}`);
  } catch (err) {
    console.error(`[sidecar] uv could not be fetched: ${err.message}`);
    console.error("[sidecar] the application needs it to install the Python engines");
    process.exit(1);
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
}

fetchUv();
