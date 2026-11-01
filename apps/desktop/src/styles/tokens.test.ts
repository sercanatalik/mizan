import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const css = readFileSync(resolve(dirname(fileURLToPath(import.meta.url)), "tokens.css"), "utf8");
const tokens = Object.fromEntries([...css.matchAll(/--([\w-]+):\s*(#[0-9a-f]{6})/gi)].map((m) => [m[1], m[2]]));

function luminance(hex: string): number {
  const [r, g, b] = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255).map((c) =>
    c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4,
  );
  return 0.2126 * r! + 0.7152 * g! + 0.0722 * b!;
}

function contrast(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi! + 0.05) / (lo! + 0.05);
}

const pairs: [string, string][] = [
  ["text", "surface"],
  ["text", "ground"],
  ["text-secondary", "surface"],
  ["text-muted", "surface"],
  ["text-muted", "ground"],
  ["text-muted", "surface-sunken"],
  ["sidebar-text", "ink-900"],
  ["sidebar-muted", "ink-900"],
  ["accent-on-dark", "ink-900"],
  ["accent-on-dark-text", "accent-on-dark"],
  ["on-accent", "accent"],
  ["accent", "surface"],
  ["accent-strong", "accent-soft"],
  ["warn", "surface"],
  ["warn-strong", "warn-soft"],
  ["info", "info-soft"],
  ["gain", "surface"],
  ["loss", "surface"],
];

test.each(pairs)("%s on %s meets 4.5:1", (fg, bg) => {
  expect(tokens[fg], fg).toBeDefined();
  expect(tokens[bg], bg).toBeDefined();
  expect(contrast(tokens[fg]!, tokens[bg]!)).toBeGreaterThanOrEqual(4.5);
});
