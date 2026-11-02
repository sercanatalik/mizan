import { periodStart } from "./view";

test.each([
  ["one_month", "2026-11-02", "2026-10-03"],
  ["three_months", "2026-11-02", "2026-08-03"],
  ["ytd", "2026-11-02", "2026-01-01"],
  ["last12_months", "2026-11-02", "2025-11-03"],
  ["one_month", "2026-03-31", "2026-03-01"],
  ["since_start", "2026-11-02", null],
] as const)("period %s ending %s starts %s", (period, today, start) => {
  expect(periodStart(period, today)).toBe(start);
});
