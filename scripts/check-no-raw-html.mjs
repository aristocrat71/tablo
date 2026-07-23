// Guard: session-derived text must never be rendered as HTML/code.
//
// Tablo's windows display strings it doesn't control — project names, paths,
// branches, session titles, commands, tool inputs. Svelte's `{value}` escapes
// them, so they show as literal text. This guard fails the build if anyone
// introduces a raw-HTML / code-exec sink in the app frontend, which could turn a
// crafted string (e.g. a folder named `<img src=x onerror=…>`) into script
// running inside a privileged webview. See issue #46.
//
// If a sink is ever genuinely needed on trusted, non-session content, mark that
// exact line with a trailing `// safe-html-ok` comment to opt it out.

import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

const ROOT = "src";
const EXTS = [".svelte", ".ts", ".js"];
const SINKS = [
  { re: /\{@html\b/, name: "{@html}" },
  { re: /\.innerHTML\b/, name: ".innerHTML" },
  { re: /\.outerHTML\b/, name: ".outerHTML" },
  { re: /insertAdjacentHTML\b/, name: "insertAdjacentHTML" },
  { re: /document\.write\b/, name: "document.write" },
  { re: /\beval\s*\(/, name: "eval(" },
  { re: /\bnew\s+Function\s*\(/, name: "new Function(" },
];

function* walk(dir) {
  for (const entry of readdirSync(dir)) {
    const p = join(dir, entry);
    if (statSync(p).isDirectory()) yield* walk(p);
    else if (EXTS.some((e) => p.endsWith(e))) yield p;
  }
}

const hits = [];
for (const file of walk(ROOT)) {
  const lines = readFileSync(file, "utf8").split("\n");
  lines.forEach((line, i) => {
    if (line.includes("safe-html-ok")) return;
    for (const sink of SINKS) {
      if (sink.re.test(line)) hits.push(`${file}:${i + 1}  ${sink.name}`);
    }
  });
}

if (hits.length) {
  console.error("Unsafe HTML/code sink(s) found — session text could become executable:");
  for (const h of hits) console.error("  " + h);
  console.error("\nRender values with Svelte's `{value}` (auto-escaped), or mark a");
  console.error("trusted line with `// safe-html-ok`. See scripts/check-no-raw-html.mjs.");
  process.exit(1);
}
console.log("check-no-raw-html: no unsafe sinks in src/ ✓");
