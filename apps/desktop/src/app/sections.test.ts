import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { SECTIONS, sectionForPath } from "./sections";

test("sections.json is up to date (run `npm run gen:sections`)", () => {
  const file = resolve(dirname(fileURLToPath(import.meta.url)), "../../src-tauri/sections.json");
  const json = JSON.parse(readFileSync(file, "utf8"));
  expect(json).toEqual(SECTIONS.map((s) => ({ id: s.id, label: s.label, shortcut: s.shortcut })));
});

test("account pages belong to Accounts", () => {
  expect(sectionForPath("/accounts/4")?.id).toBe("accounts");
  expect(sectionForPath("/")?.id).toBe("overview");
  expect(sectionForPath("/market-data")?.id).toBe("market-data");
});
