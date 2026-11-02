import { compact } from "./format";

test.each([
  ["279949.48", "279.95K"],
  ["4812350.00", "4.81M"],
  ["1250000000", "1.25B"],
  ["999.50", "999.50"],
  ["0.00", "0.00"],
  ["+402180.00", "+402.18K"],
  ["-1284.60", "−1.28K"],
])("compact(%s) is %s", (plain, short) => {
  expect(compact(plain)).toBe(short);
});
