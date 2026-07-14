import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { existsSync, mkdirSync, writeFileSync } from "node:fs";

// The repo root tsconfig.json extends ../.svelte-kit/tsconfig.json — a gitignored
// file that `svelte-kit sync` only generates locally. On a fresh CI clone it's
// absent, so Astro's bundler walks up, reads the root tsconfig, and aborts on the
// missing `extends` target. Drop a minimal stub if (and only if) one isn't already
// present, so a real generated tsconfig is never clobbered.
const here = dirname(fileURLToPath(import.meta.url)); // tablo-website/scripts
const target = resolve(here, "../../.svelte-kit/tsconfig.json"); // <repo>/.svelte-kit/tsconfig.json

if (!existsSync(target)) {
  mkdirSync(dirname(target), { recursive: true });
  writeFileSync(target, "{}\n");
  console.log("[ensure-root-tsconfig] wrote stub:", target);
}
