import type { ImportView } from "../bridge/generated/ImportView";
import type { ReviewReport } from "../bridge/generated/ReviewReport";

const SHA_A = "a".repeat(64);
const SHA_B = "b".repeat(64);

const rec = (index: number, locator: string, kind: string, date: string, instrument: string | null, quantity: string | null, net: string | null) => ({
  index,
  locator,
  kind,
  date,
  instrument,
  quantity,
  net_cash: net,
  memo: null,
});

/** A review mirroring the core e2e fixture: one undecided ASML buy, a figure warning, a diagnostic. */
export function reviewView(decided: "new" | "duplicate" | null = null, overrides: Partial<ReviewReport> = {}): ImportView {
  const asml = rec(0, "row 12", "buy", "2026-07-01", "isin:NL0010273215", "2", "1284.60 EUR");
  const newRows = [
    rec(2, "row 25", "sell", "2026-07-01", "isin:NL0010273215", "1", "684.20 EUR"),
    rec(3, "row 131", "sell", "2026-09-12", "isin:DE0007164600", "8", "1702.10 EUR"),
  ];
  const passed = decided === "new";
  const report: ReviewReport = {
    summary: {
      new: 38 + (decided === "new" ? 1 : 0),
      duplicates: 101 + (decided === "duplicate" ? 1 : 0),
      pending_confirmations: decided ? 0 : 2,
      consistency_warnings: 1,
      ordering_diagnostics: 0,
      adapter_diagnostics: 1,
      validation_errors: 0,
      unresolved: 0,
      assertions_failed: passed ? 0 : 1,
    },
    new_records: decided === "new" ? [asml, ...newRows] : newRows,
    duplicates: [rec(1, "row 20", "dividend", "2026-07-03", "isin:IE00BK5BQT80", null, "52.10 EUR"), ...(decided === "duplicate" ? [asml] : [])],
    confirmations: [{ record: asml, existing_activity_id: 2, existing_file_sha256: SHA_A, existing_locator: "row 188", decision: decided }],
    consistency_warnings: [
      { index: 3, locator: "row 131", message: "quantity × price × multiplier − charges = 1712.10 EUR, but the statement's net cash is 1702.10 EUR (10 EUR apart)" },
    ],
    ordering_diagnostics: [],
    adapter_diagnostics: [{ severity: "warning", message: "No rule for description \"Zinsgutschrift Korrektur\"", locator: "row 87", raw: undefined }],
    validation_errors: [],
    unresolved: [],
    assertion_results: [
      {
        account: "DEG-4417",
        as_of: "2026-09-30",
        locator: "summary",
        passed,
        note: null,
        lines: [
          { label: "EUR", expected: "4869.61", computed: passed ? "4869.61" : "6154.21", passed },
          { label: "isin:NL0010273215", expected: "3", computed: passed ? "3" : "1", passed },
        ],
      },
    ],
    flagged_adjustments: [],
    basis_unknown: [],
    ...overrides,
  };
  return {
    file_name: "degiro-2026Q3.json",
    source: { adapter: null, matches: [], adapters: ["degiro-csv 0.4.0"], file_sha256: SHA_B, accounts: ["DEG-4417"], first_date: "2026-07-01", last_date: "2026-09-30" },
    report,
  };
}
