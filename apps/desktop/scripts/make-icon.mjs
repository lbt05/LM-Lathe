/**
 * Puts the logo of the application into the place Tauri expects.
 *
 * The artwork lives in `src-tauri/logo.png` (1024×1024, rounded corners,
 * transparent outside). It is copied to `src-tauri/icons/icon.png`, and
 * `tauri icon` derives every other size from it.
 */
import { copyFileSync, existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const tauri = join(here, "..", "src-tauri");
const source = join(tauri, "logo.png");
const destination = join(tauri, "icons", "icon.png");

if (!existsSync(source)) {
  console.error(`[icon] ${source} is missing`);
  process.exit(1);
}

copyFileSync(source, destination);
console.log(`[icon] ${source} -> ${destination}`);
