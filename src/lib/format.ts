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
