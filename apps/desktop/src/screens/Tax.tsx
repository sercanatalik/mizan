import { open } from "@tauri-apps/plugin-dialog";
import { useState } from "react";
import { PageHeader } from "../app/Layout";
import { errorMessage, type TaxTreatment } from "../bridge/api";
import type { Country } from "../bridge/generated/Country";
import type { GainRowView } from "../bridge/generated/GainRowView";
import type { GainSection } from "../bridge/generated/GainSection";
import type { IncomeSection } from "../bridge/generated/IncomeSection";
import type { TaxReport } from "../bridge/generated/TaxReport";
import { useExportTaxCsv, useSetTaxTreatment, useTaxReport, useTaxYears } from "../bridge/queries";
import { Button, Card, DataTable, EmptyState, Segmented } from "../ui";
import { amount, longDate } from "./format";

const COUNTRIES: { value: Country; label: string }[] = [
  { value: "uk", label: "UK" },
  { value: "us", label: "US" },
  { value: "tr", label: "Turkey" },
];

const TREATMENTS: { value: TaxTreatment; label: string }[] = [
  { value: "taxable", label: "Taxable" },
  { value: "isa", label: "ISA (tax-free)" },
  { value: "pension", label: "Pension" },
];

const muted = { fontSize: 12, color: "var(--text-muted)" } as const;
const right = { textAlign: "right" } as const;
const select = { font: "inherit", padding: "4px 8px", borderRadius: 6, border: "1px solid var(--border-strong)" } as const;

/** A signed amount; losses in the loss colour. */
function Money({ value }: { value: string | null }) {
  if (value === null) return <span style={muted}>—</span>;
  return (
    <span className="num" style={{ color: value.startsWith("-") ? "var(--loss)" : undefined }}>
      {amount(value)}
    </span>
  );
}

/** The gain cell: the amount, or why there is none. */
function GainCell({ row }: { row: GainRowView }) {
  if (row.reason) {
    return (
      <span style={{ fontSize: 12, color: "var(--warn-strong)" }} data-testid="row-reason">
        {row.reason}
      </span>
    );
  }
  return <Money value={row.gain} />;
}

function Instrument({ row }: { row: GainRowView }) {
  return (
    <span style={{ display: "inline-flex", flexDirection: "column", gap: 2 }}>
      <span style={{ fontWeight: 500 }}>{row.description}</span>
      {row.isin && <span style={muted}>{row.isin}</span>}
      {row.notes.map((n) => (
        <span key={n} style={{ fontSize: 12, color: "var(--warn-strong)" }}>
          {n}
        </span>
      ))}
    </span>
  );
}

function date(iso: string | null): string {
  return iso ? longDate(iso) : "—";
}

function GainTable({ country, section }: { country: Country; section: GainSection }) {
  const rows = section.rows;
  if (country === "uk") {
    return (
      <DataTable label={section.title} minWidth={860}>
        <thead>
          <tr>
            <th scope="col">Date sold</th>
            <th scope="col">Instrument</th>
            <th scope="col">Account</th>
            <th scope="col" style={right}>Quantity</th>
            <th scope="col">Rule</th>
            <th scope="col" style={right}>Proceeds</th>
            <th scope="col" style={right}>Allowable cost</th>
            <th scope="col" style={right}>Gain or loss</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r, i) => (
            <tr key={i} data-testid="gain-row">
              <td className="num">{date(r.date_sold)}</td>
              <td><Instrument row={r} /></td>
              <td>{r.account ?? "—"}</td>
              <td className="num" style={right}>{r.quantity}</td>
              <td>{r.rule ? (r.rule === "30 days" ? `30 days (bought ${date(r.date_acquired)})` : r.rule) : "—"}</td>
              <td style={right}><Money value={r.proceeds} /></td>
              <td style={right}><Money value={r.cost} /></td>
              <td style={right}><GainCell row={r} /></td>
            </tr>
          ))}
        </tbody>
      </DataTable>
    );
  }
  if (country === "us") {
    return (
      <DataTable label={section.title} minWidth={900}>
        <thead>
          <tr>
            <th scope="col">(a) Description</th>
            <th scope="col">(b) Acquired</th>
            <th scope="col">(c) Sold</th>
            <th scope="col" style={right}>(d) Proceeds</th>
            <th scope="col" style={right}>(e) Cost basis</th>
            <th scope="col">(f) Code</th>
            <th scope="col" style={right}>(g) Adjustment</th>
            <th scope="col" style={right}>(h) Gain or loss</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r, i) => (
            <tr key={i} data-testid="gain-row">
              <td>
                <Instrument row={r} />
                <span style={{ ...muted, display: "block" }}>
                  {r.quantity} · {r.account ?? "—"}
                </span>
              </td>
              <td className="num">{date(r.date_acquired)}</td>
              <td className="num">{date(r.date_sold)}</td>
              <td style={right}><Money value={r.proceeds} /></td>
              <td style={right}><Money value={r.cost} /></td>
              <td data-testid="adjustment-code">{r.adjustment_code ?? ""}</td>
              <td style={right}>{r.adjustment ? <Money value={r.adjustment} /> : ""}</td>
              <td style={right}><GainCell row={r} /></td>
            </tr>
          ))}
        </tbody>
      </DataTable>
    );
  }
  return (
    <DataTable label={section.title} minWidth={960}>
      <thead>
        <tr>
          <th scope="col">Instrument</th>
          <th scope="col">Account</th>
          <th scope="col" style={right}>Quantity</th>
          <th scope="col">Acquired</th>
          <th scope="col">Sold</th>
          <th scope="col" style={right}>Cost</th>
          <th scope="col" style={right}>Indexed cost</th>
          <th scope="col" style={right}>Proceeds</th>
          <th scope="col" style={right}>Gain or loss</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((r, i) => (
          <tr key={i} data-testid="gain-row">
            <td><Instrument row={r} /></td>
            <td>{r.account ?? "—"}</td>
            <td className="num" style={right}>{r.quantity}</td>
            <td className="num">{date(r.date_acquired)}</td>
            <td className="num">{date(r.date_sold)}</td>
            <td style={right}><Money value={r.cost} /></td>
            <td style={right}>{r.indexed_cost ? <Money value={r.indexed_cost} /> : <span style={muted}>—</span>}</td>
            <td style={right}><Money value={r.proceeds} /></td>
            <td style={right}><GainCell row={r} /></td>
          </tr>
        ))}
      </tbody>
    </DataTable>
  );
}

function Gains({ report }: { report: TaxReport }) {
  const shown = report.gains.filter((s) => s.counted || s.rows.length > 0);
  return (
    <>
      {shown.map((s) => (
        <Card
          key={s.id}
          title={s.title}
          label={s.title}
          aside={
            s.rows.length > 0 && (
              <span style={muted} data-testid="section-total">
                {s.counted ? "Net" : "Not counted · net"} {amount(s.totals.net)} {report.currency}
              </span>
            )
          }
        >
          {s.note && <p style={{ ...muted, margin: 0 }}>{s.note}</p>}
          {s.rows.length === 0 ? <p style={{ ...muted, margin: 0 }}>No disposals in this tax year.</p> : <GainTable country={report.country} section={s} />}
        </Card>
      ))}
    </>
  );
}

function Income({ report }: { report: TaxReport }) {
  const shown = report.income.filter((s: IncomeSection) => s.counted || s.rows.length > 0);
  return (
    <>
      {shown.map((s) => (
        <Card key={`income-${s.id}`} title={s.title} label={s.title}>
          {s.note && <p style={{ ...muted, margin: 0 }}>{s.note}</p>}
          {s.rows.length === 0 ? (
            <p style={{ ...muted, margin: 0 }}>No dividends or interest in this tax year.</p>
          ) : (
            <DataTable label={s.title} minWidth={760}>
              <thead>
                <tr>
                  <th scope="col">Date</th>
                  <th scope="col">Type</th>
                  <th scope="col">Instrument</th>
                  <th scope="col">Account</th>
                  <th scope="col" style={right}>Gross</th>
                  <th scope="col" style={right}>Tax withheld</th>
                </tr>
              </thead>
              <tbody>
                {s.rows.map((r, i) => (
                  <tr key={i} data-testid="income-row">
                    <td className="num">{longDate(r.date)}</td>
                    <td>{r.kind}</td>
                    <td>
                      {r.description}
                      {r.isin && <span style={{ ...muted, display: "block" }}>{r.isin}</span>}
                    </td>
                    <td>{r.account ?? "—"}</td>
                    <td style={right}>
                      {r.reason ? (
                        <span style={{ fontSize: 12, color: "var(--warn-strong)" }} data-testid="row-reason">
                          {r.reason}
                        </span>
                      ) : (
                        <Money value={r.gross} />
                      )}
                    </td>
                    <td style={right}><Money value={r.withheld} /></td>
                  </tr>
                ))}
                {s.totals.map((t) => (
                  <tr key={t.kind} data-testid="income-total">
                    <td colSpan={4} style={{ fontWeight: 600 }}>
                      Total {t.kind}
                    </td>
                    <td style={{ ...right, fontWeight: 600 }}><Money value={t.gross} /></td>
                    <td style={{ ...right, fontWeight: 600 }}><Money value={t.withheld} /></td>
                  </tr>
                ))}
              </tbody>
            </DataTable>
          )}
        </Card>
      ))}
    </>
  );
}

function Accounts({ report }: { report: TaxReport }) {
  const set = useSetTaxTreatment();
  return (
    <Card title="Accounts" label="Account tax treatment" aside={<span style={muted}>ISA and pension accounts are left out of every report</span>}>
      <ul style={{ listStyle: "none", margin: 0, padding: 0, display: "grid", gap: 8 }}>
        {report.accounts.map((a) => (
          <li key={a.id} style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: 12 }}>
            <span style={{ fontWeight: 500 }}>{a.name}</span>
            <select
              aria-label={`Tax treatment of ${a.name}`}
              value={a.treatment}
              disabled={set.isPending}
              onChange={(e) => set.mutate({ accountId: a.id, treatment: e.target.value as TaxTreatment })}
              style={select}
            >
              {TREATMENTS.map((t) => (
                <option key={t.value} value={t.value}>
                  {t.label}
                </option>
              ))}
            </select>
          </li>
        ))}
      </ul>
      {set.error && <p style={{ fontSize: 12, color: "var(--loss)", margin: 0 }}>{errorMessage(set.error)}</p>}
    </Card>
  );
}

function ExportButton({ country, year }: { country: Country; year: string }) {
  const exp = useExportTaxCsv();
  const run = async () => {
    const dir = await open({ directory: true, title: "Choose a folder for the CSV files" });
    if (typeof dir !== "string") return;
    exp.mutate({ country, year, dir });
  };
  return (
    <span style={{ display: "inline-flex", flexDirection: "column", alignItems: "flex-end", gap: 2 }}>
      <Button onClick={() => void run()} disabled={exp.isPending}>
        Export CSV
      </Button>
      {exp.data && (
        <span style={muted} data-testid="export-done">
          Saved {exp.data.gains.split(/[\\/]/).pop()} and {exp.data.income.split(/[\\/]/).pop()}
        </span>
      )}
      {exp.error && <span style={{ fontSize: 12, color: "var(--loss)" }}>{errorMessage(exp.error)}</span>}
    </span>
  );
}

function Report({ report }: { report: TaxReport }) {
  const notComputed = report.not_computed.reduce((n, r) => n + r.count, 0);
  return (
    <>
      <p role="note" data-testid="tax-notice" style={{ margin: 0, padding: "10px 12px", borderRadius: 8, background: "var(--surface-sunken, #f5f5f4)", fontSize: 13 }}>
        {report.notice}
      </p>
      {report.incomplete && (
        <p role="status" data-testid="not-computed" style={{ margin: 0, fontSize: 13, color: "var(--warn-strong)" }}>
          {notComputed} {notComputed === 1 ? "row" : "rows"} not computed: {report.not_computed.map((r) => `${r.reason} (${r.count})`).join(" · ")}. The totals
          leave {notComputed === 1 ? "it" : "them"} out.
        </p>
      )}
      <section aria-label="Summary" data-testid="summary" style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(min(170px, 100%), 1fr))", gap: 12 }}>
        {report.summary.map((f) => (
          <Card key={f.label} label={f.label}>
            <span style={muted}>{f.label}</span>
            <span className="num" style={{ fontSize: 20, fontWeight: 500 }} data-testid={`figure-${f.label}`}>
              {f.label === "Disposals" ? f.value : `${amount(f.value)} ${report.currency}`}
            </span>
          </Card>
        ))}
      </section>
      <Gains report={report} />
      <Income report={report} />
      {report.notes.length > 0 && (
        <ul style={{ ...muted, margin: 0, paddingLeft: 18 }} data-testid="tax-notes">
          {report.notes.map((n) => (
            <li key={n}>{n}</li>
          ))}
        </ul>
      )}
      <p style={{ ...muted, margin: 0 }}>
        Exchange rates: {report.rate_source}. Computed {report.computed_at.replace("T", " ").replace("Z", " UTC")}.
      </p>
    </>
  );
}

export function Tax() {
  const [country, setCountry] = useState<Country>("uk");
  const [picked, setPicked] = useState<Record<string, string>>({});
  const { data: years } = useTaxYears(country);
  const year = picked[country] ?? years?.default ?? years?.current ?? null;
  const { data: report } = useTaxReport(country, year);
  const option = years?.years.find((y) => y.label === year);
  const empty = years && years.years.length === 0;
  return (
    <>
      <PageHeader
        title="Tax"
        subtitle={option && report ? `Tax year ${option.label} · ${longDate(option.first_day)} – ${longDate(option.last_day)} · ${report.currency}` : undefined}
        actions={
          <>
            <Segmented label="Country" value={country} onChange={setCountry} options={COUNTRIES} />
            {years && years.years.length > 0 && (
              <label style={{ display: "inline-flex", alignItems: "center", gap: 8, fontSize: 13 }}>
                Tax year
                <select value={year ?? ""} onChange={(e) => setPicked({ ...picked, [country]: e.target.value })} style={select}>
                  {years.years.map((y) => (
                    <option key={y.label} value={y.label}>
                      {y.label}
                    </option>
                  ))}
                </select>
              </label>
            )}
            {!empty && year && <ExportButton country={country} year={year} />}
          </>
        }
      />
      {empty ? (
        <EmptyState
          title="No taxable activity yet"
          message="Tax reports list sales, dividends and interest in taxable accounts. Import a statement batch, or mark an account as taxable below."
          action={{ to: "/imports", label: "Go to Imports" }}
        />
      ) : (
        report && <Report report={report} />
      )}
      {report && report.accounts.length > 0 && <Accounts report={report} />}
    </>
  );
}
