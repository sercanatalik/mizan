// Turns the core review report into rows for the records table.
import type { Decision } from "../../bridge/generated/Decision";
import type { Diagnostic } from "../../bridge/generated/Diagnostic";
import type { RecordSummary } from "../../bridge/generated/RecordSummary";
import type { ReviewReport } from "../../bridge/generated/ReviewReport";

export type RowStatus = "new" | "duplicate" | "confirm" | "decided_new" | "decided_duplicate";

export interface RecordRow {
  kind: "record";
  record: RecordSummary;
  status: RowStatus;
  /** Set for records that need (or had) a decision. */
  confirmation?: { existingLocator: string; existingFile: string; decision: Decision | null };
  warnings: string[];
}

export interface DiagnosticRow {
  kind: "diagnostic";
  diagnostic: Diagnostic;
}

export type Row = RecordRow | DiagnosticRow;
export type Filter = "attention" | "new" | "duplicates" | "all";

export function buildRows(r: ReviewReport): Row[] {
  const warnings = new Map<number, string[]>();
  for (const w of r.consistency_warnings) warnings.set(w.index, [...(warnings.get(w.index) ?? []), w.message]);

  const rows = new Map<number, RecordRow>();
  for (const rec of r.new_records) rows.set(rec.index, { kind: "record", record: rec, status: "new", warnings: warnings.get(rec.index) ?? [] });
  for (const rec of r.duplicates) {
    if (!rows.has(rec.index)) rows.set(rec.index, { kind: "record", record: rec, status: "duplicate", warnings: warnings.get(rec.index) ?? [] });
  }
  for (const c of r.confirmations) {
    const status: RowStatus = c.decision === "new" ? "decided_new" : c.decision === "duplicate" ? "decided_duplicate" : "confirm";
    rows.set(c.record.index, {
      kind: "record",
      record: c.record,
      status,
      confirmation: { existingLocator: c.existing_locator, existingFile: c.existing_file_sha256, decision: c.decision ?? null },
      warnings: warnings.get(c.record.index) ?? [],
    });
  }
  const records = [...rows.values()].sort((a, b) => a.record.index - b.record.index);
  const diagnostics: DiagnosticRow[] = r.adapter_diagnostics.map((d) => ({ kind: "diagnostic", diagnostic: d }));
  return [...records, ...diagnostics];
}

export function matches(row: Row, f: Filter): boolean {
  if (row.kind === "diagnostic") return f === "attention" || f === "all";
  switch (f) {
    case "attention":
      return row.status === "confirm" || row.warnings.length > 0;
    case "new":
      return row.status === "new" || row.status === "decided_new";
    case "duplicates":
      return row.status === "duplicate" || row.status === "decided_duplicate";
    case "all":
      return true;
  }
}

export function counts(rows: Row[]): Record<Filter, number> {
  const f: Filter[] = ["attention", "new", "duplicates", "all"];
  return Object.fromEntries(f.map((x) => [x, rows.filter((r) => matches(r, x)).length])) as Record<Filter, number>;
}
