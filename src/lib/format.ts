import type { ActivityKind } from "./types";

// Terminal (dashboard) prompt marker: "#" for the human's own prompt, ">" for
// anything the agent does (tool call / spoken output / thinking). Two markers,
// matching the legend; the tool/text/think distinction is carried by color.
// Typographic punctuation only — never an emoji.
export function activityMark(kind: string): string {
  return kind === "user" ? "#" : ">";
}

// Terminal titlebar status label.
export function terminalStatus(k: ActivityKind): string {
  return k === "working"
    ? "working"
    : k === "waiting"
      ? "waiting for input"
      : k === "thinking"
        ? "thinking"
        : "idle";
}

// Compact token counts: 46000 -> "46k", 1000000 -> "1M".
export function tokens(n: number): string {
  if (n >= 1_000_000) {
    const m = n / 1_000_000;
    return `${m >= 10 || Number.isInteger(m) ? Math.round(m) : m.toFixed(1)}M`;
  }
  if (n >= 1_000) return `${Math.round(n / 1000)}k`;
  return `${n}`;
}

// Percent for display: whole numbers, no trailing ".0".
export function pct(n: number): string {
  return `${Math.round(n)}%`;
}

// "42s" / "4m12s" / "1h05m". Clamped at 0 so transcript-vs-local clock skew
// can't render a negative age.
export function elapsed(sinceMs: number, nowMs: number): string {
  const secs = Math.max(0, Math.floor((nowMs - sinceMs) / 1000));
  if (secs < 60) return `${secs}s`;
  const mins = Math.floor(secs / 60);
  const pad = (n: number) => String(n).padStart(2, "0");
  return mins < 60 ? `${mins}m${pad(secs % 60)}s` : `${Math.floor(mins / 60)}h${pad(mins % 60)}m`;
}

// Friendly label for the raw account tier from ~/.claude.json, e.g.
// "default_claude_max_5x" -> "Max 5×". Strips the vendor prefix, turns the
// "_5x" suffix into "5×", title-cases the rest, and falls back gracefully for
// unknown tier strings. Returns null when there's nothing to show.
export function planTier(raw: string | null): string | null {
  if (!raw) return null;
  const cleaned = raw
    .replace(/^default_/, "")
    .replace(/^claude_/, "")
    .replace(/_(\d+)x\b/g, " $1×")
    .replace(/_/g, " ")
    .trim();
  if (!cleaned) return null;
  return cleaned
    .split(" ")
    .map((w) => (/^\d+×$/.test(w) ? w : w.charAt(0).toUpperCase() + w.slice(1)))
    .join(" ");
}
