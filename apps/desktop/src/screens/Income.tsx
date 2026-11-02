import { useMutation } from "@tanstack/react-query";
import { open } from "@tauri-apps/plugin-dialog";
import { useNavigate } from "react-router";
import { PageHeader } from "../app/Layout";
import { PERIODS, useScopeName, useView } from "../app/view";
import { api, errorMessage } from "../bridge/api";
import type { IncomeInstrument } from "../bridge/generated/IncomeInstrument";
import type { IncomeMonth } from "../bridge/generated/IncomeMonth";
import type { IncomeTotals } from "../bridge/generated/IncomeTotals";
import type { IncomeView } from "../bridge/generated/IncomeView";
import type { Period } from "../bridge/generated/Period";
import { useIncome } from "../bridge/queries";
import { Button, Card, DataTable, EmptyState } from "../ui";
import { amount, longDate } from "./format";

const INCOME_VIEW = { scope: true, period: true, currency: true } as const;
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const muted = { fontSize: 12, color: "var(--text-muted)" } as const;
const right = { textAlign: "right" } as const;

function monthLabel(ym: string): string {
  const [y, m] = ym.split("-");
  return `${MONTHS[Number(m) - 1] ?? m} ${y}`;
}

function SummaryCard({ title, value, previous, currency }: { title: string; value: string; previous: string | null; currency: string }) {
  return (
    <Card label={title}>
      <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
        <span style={muted}>{title}</span>
        <span className="num" style={{ fontSize: 22, fontWeight: 500, letterSpacing: "-0.02em" }}>
          {amount(value)} {currency}
        </span>
        {previous !== null && <span style={muted}>Previous period: {amount(previous)}</span>}
      </div>
    </Card>
  );
}

function Summary({ v }: { v: IncomeView }) {
  const prev = (f: (t: IncomeTotals) => string) => (v.previous ? f(v.previous) : null);
  return (
    <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(220px, 1fr))", gap: 14 }}>
      <SummaryCard title="Gross income" value={v.totals.gross} previous={prev((t) => t.gross)} currency={v.base_currency} />
      <SummaryCard title="Tax withheld" value={v.totals.withheld} previous={prev((t) => t.withheld)} currency={v.base_currency} />
      <SummaryCard title="Net income" value={v.totals.net} previous={prev((t) => t.net)} currency={v.base_currency} />
    </div>
  );
}

/** Net income per month as bars, dividends below interest; empty months stay empty. */
function MonthlyChart({ months, currency }: { months: IncomeMonth[]; currency: string }) {
  const values = months.map((m) => ({ m, d: Math.max(0, Number(m.dividends)), i: Math.max(0, Number(m.interest)) }));
  const max = Math.max(1, ...values.map((x) => x.d + x.i));
  const w = 100 / Math.max(1, values.length);
  const summary = `Net income by month, ${monthLabel(months[0]!.month)} to ${monthLabel(months[months.length - 1]!.month)}. ${values
    .map((x) => `${monthLabel(x.m.month)}: ${x.d + x.i === 0 ? "none" : `${(x.d + x.i).toFixed(2)} ${currency}`}`)
    .join("; ")}.`;
  return (
    <Card
      title="By month"
      label="Income by month"
      aside={
        <span style={{ ...muted, display: "inline-flex", gap: 12 }}>
          <span>
            <span aria-hidden="true" style={{ display: "inline-block", width: 10, height: 10, borderRadius: 2, background: "var(--accent)", marginRight: 4 }} />
            Dividends and distributions
          </span>
          <span>
            <span aria-hidden="true" style={{ display: "inline-block", width: 10, height: 10, borderRadius: 2, background: "var(--info)", marginRight: 4 }} />
            Interest
          </span>
        </span>
      }
    >
      <svg role="img" aria-label={summary} viewBox="0 0 100 40" preserveAspectRatio="none" style={{ width: "100%", height: 200, display: "block" }}>
        {values.map((x, k) => {
          const hd = (x.d / max) * 38;
          const hi = (x.i / max) * 38;
          return (
            <g key={x.m.month}>
              <rect x={k * w + w * 0.15} y={40 - hd} width={w * 0.7} height={hd} fill="var(--accent)" />
              <rect x={k * w + w * 0.15} y={40 - hd - hi} width={w * 0.7} height={hi} fill="var(--info)" />
            </g>
          );
        })}
      </svg>
      <div style={{ display: "flex", justifyContent: "space-between", ...muted }} aria-hidden="true">
        <span>{monthLabel(months[0]!.month)}</span>
        {months.length > 2 && <span>{monthLabel(months[Math.floor((months.length - 1) / 2)]!.month)}</span>}
        <span>{monthLabel(months[months.length - 1]!.month)}</span>
      </div>
    </Card>
  );
}

function Instruments({ v }: { v: IncomeView }) {
  const navigate = useNavigate();
  const openActivity = (r: IncomeInstrument) => {
    const q = new URLSearchParams({ account: String(r.account_id) });
    if (r.instrument_key) {
      q.set("text", r.label);
      q.set("kinds", "dividend,distribution");
    } else {
      q.set("kinds", "interest");
    }
    if (v.from) q.set("from", v.from);
    q.set("to", v.to);
    navigate(`/activity?${q.toString()}`);
  };
  return (
    <Card title="By instrument" label="Income by instrument">
      <DataTable label="Income by instrument" minWidth={680}>
        <thead>
          <tr>
            <th scope="col">Instrument</th>
            <th scope="col">Account</th>
            <th scope="col" style={right}>Payments</th>
            <th scope="col" style={right}>Gross</th>
            <th scope="col" style={right}>Withheld</th>
            <th scope="col" style={right}>Net ({v.base_currency})</th>
          </tr>
        </thead>
        <tbody>
          {v.instruments.map((r) => (
            <tr key={`${r.instrument_key}-${r.account_id}-${r.label}`} data-testid="income-row">
              <td>
                <button
                  type="button"
                  onClick={() => openActivity(r)}
                  style={{ border: 0, padding: 0, background: "none", font: "inherit", fontWeight: 500, color: "var(--accent-strong)", cursor: "pointer", textAlign: "left" }}
                  aria-label={`Show ${r.label} payments in ${r.account}`}
                >
                  {r.label}
                </button>
              </td>
              <td style={{ color: "var(--text-secondary)" }}>{r.account}</td>
              <td className="num" style={right}>
                {r.payments}
              </td>
              <td className="num" style={right}>
                {amount(r.gross)}
              </td>
              <td className="num" style={right}>
                {amount(r.withheld)}
              </td>
              <td className="num" style={{ ...right, fontWeight: 500 }}>
                {amount(r.net)}
              </td>
            </tr>
          ))}
        </tbody>
      </DataTable>
    </Card>
  );
}

function Payments({ v }: { v: IncomeView }) {
  return (
    <Card title="Payments" label="Payments">
      <DataTable label="Payments" minWidth={680}>
        <thead>
          <tr>
            <th scope="col">Date</th>
            <th scope="col">Instrument</th>
            <th scope="col">Account</th>
            <th scope="col" style={right}>Net</th>
            <th scope="col" style={right}>Net ({v.base_currency})</th>
          </tr>
        </thead>
        <tbody>
          {[...v.payments].reverse().map((p, i) => (
            <tr key={`${p.date}-${p.account_id}-${p.label}-${i}`} data-testid="payment-row">
              <td className="num">{p.date}</td>
              <td>{p.label}</td>
              <td style={{ color: "var(--text-secondary)" }}>{p.account}</td>
              <td className="num" style={right}>
                {p.net ? `${amount(p.net)} ${p.currency ?? ""}` : "—"}
              </td>
              <td className="num" style={right}>
                {p.base_net ? amount(p.base_net) : <span style={{ ...muted, color: "var(--warn-strong)" }}>{p.note}</span>}
              </td>
            </tr>
          ))}
        </tbody>
      </DataTable>
    </Card>
  );
}

function ExportButton({ period, accountId }: { period: Period; accountId: number | null }) {
  const exp = useMutation({ mutationFn: (dir: string) => api.exportIncomeCsv(period, accountId, dir) });
  const run = async () => {
    const dir = await open({ directory: true, title: "Choose a folder for the CSV file" });
    if (typeof dir !== "string") return;
    exp.mutate(dir);
  };
  return (
    <span style={{ display: "inline-flex", flexDirection: "column", alignItems: "flex-end", gap: 2 }}>
      <Button onClick={() => void run()} disabled={exp.isPending}>
        Export CSV
      </Button>
      {exp.data && (
        <span style={muted} data-testid="export-done">
          Saved {exp.data.split(/[\\/]/).pop()}
        </span>
      )}
      {exp.error && <span style={{ fontSize: 12, color: "var(--loss)" }}>{errorMessage(exp.error)}</span>}
    </span>
  );
}

export function Income() {
  const { view } = useView();
  const scopeName = useScopeName();
  const { data } = useIncome(view.period, view.account_id);
  const periodName = PERIODS.find((p) => p.value === view.period)?.long ?? "";
  if (!data) return <PageHeader title="Income" view={INCOME_VIEW} />;
  const range = data.from ? `${longDate(data.from)} – ${longDate(data.to)}` : `to ${longDate(data.to)}`;
  const empty = data.payments.length === 0;
  return (
    <>
      <PageHeader
        title="Income"
        subtitle={`${scopeName} · ${range}`}
        view={INCOME_VIEW}
        actions={!empty ? <ExportButton period={view.period} accountId={view.account_id} /> : undefined}
      />
      {empty ? (
        <EmptyState title={`No income ${periodName}`} message="Dividends, distributions and interest appear here once they are imported or added." />
      ) : (
        <>
          <Summary v={data} />
          {data.not_converted > 0 && (
            <p role="note" style={{ margin: 0, fontSize: 13, color: "var(--warn-strong)" }}>
              Totals leave out {data.not_converted} payment{data.not_converted === 1 ? "" : "s"} that could not be converted to {data.base_currency} for lack of an exchange rate.
            </p>
          )}
          {data.months.length > 0 && <MonthlyChart months={data.months} currency={data.base_currency} />}
          {data.instruments.length > 0 && <Instruments v={data} />}
          <Payments v={data} />
        </>
      )}
    </>
  );
}
