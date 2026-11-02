// Writes src-tauri/sections.json from src/app/sections.ts so the native menu
// (src-tauri/src/menu.rs) can be checked against the sidebar in a Rust test.
import { writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { SECTIONS } from "../src/app/sections.ts";

const out = resolve(dirname(fileURLToPath(import.meta.url)), "../src-tauri/sections.json");
const list = SECTIONS.map((s) => ({ id: s.id, label: s.label, shortcut: s.shortcut }));
writeFileSync(out, `${JSON.stringify(list, null, 2)}\n`);
console.log(`wrote ${list.length} sections to ${out}`);
