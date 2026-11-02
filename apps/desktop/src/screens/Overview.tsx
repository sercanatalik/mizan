import { Link, useNavigate } from "react-router";
import { PageHeader } from "../app/Layout";
import { periodStart, PERIODS, useScopeName, useView } from "../app/view";
import type { Overview as OverviewData } from "../bridge/generated/Overview";
import { FigureText } from "./Performance";
import type { AccountStatus } from "../bridge/generated/AccountStatus";
import type { Allocation } from "../bridge/generated/Allocation";
import { useIncome, useInbox, useOverview, usePerformance, useValueHistory } from "../bridge/queries";
import { Card, EmptyState } from "../ui";
import { amount, CLASS_LABELS, compact, longDate } from "./format";
import type { InboxSelectRequest } from "./Inbox";
import { NoData } from "./NoData";
import { RefreshButton } from "./RefreshButton";
import { ValueOverTime } from "./ValueOverTime";
import type { TotalView } from "../bridge/generated/TotalView";

function excluded(t: TotalView): string | null {
  const parts = [];
  if (t.excluded_no_price) parts.push(`${t.excluded_no_price} position${t.excluded_no_price === 1 ? "" : "s"} without a price`);
  if (t.excluded_no_rate) parts.push(`${t.excluded_no_rate} without an exchange rate`);
  return parts.length ? `excludes ${parts.join(" and ")}` : null;
}

function statusText(a: AccountStatus): { text: string; warn: boolean } {
  const c = a.latest_check;
  if (!c) return { text: "No balance check yet", warn: false };
  if (c.passed) return { text: `Statement to ${longDate(c.as_of)} · all balances match`, warn: false };
  const labels = [...new Set(c.failed.map((f) => f.display))].join(", ");
  const n = c.failed.length;
  return { text: `${n} balance check${n === 1 ? "" : "s"} failed · ${labels}`, warn: true };
}

const row = { display: "flex", alignItems: "flex-start", gap: 12, padding: "12px 0", borderTop: "1px solid var(--divider)" } as const;

const OVERVIEW_VIEW = { scope: true, period: true, currency: true, real: true } as const;

export function Overview() {
  const { view } = useView();
  const scopeName = useScopeName();
  const { data } = useOverview(view.account_id);
  if (!data) return <PageHeader title="Overview" view={OVERVIEW_VIEW} />;
  if (data.accounts.length === 0) {
    return (
      <>
        <PageHeader title="Overview" view={OVERVIEW_VIEW} />
        <NoData what="your accounts" />
      </>
    );
  }
  return (
    <>
      <PageHeader title="Overview" subtitle={scopeName} actions={<RefreshButton />} view={OVERVIEW_VIEW} />
      <SummaryCards data={data} />
      <ValueOverTime accountId={view.account_id} period={view.period} />
      <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(min(420px, 100%), 1fr))", gap: 14, alignItems: "start" }}>
        <Card title="Accounts" label="Accounts">
          <div>
            {data.accounts.map((a) => {
              const st = statusText(a);
              return (
                <div key={a.account_id} style={row} data-testid="account-row">
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <Link to={`/accounts/${a.account_id}`} style={{ fontWeight: 500, color: "inherit" }}>
                      {a.name}
                    </Link>
                    <div style={{ fontSize: 12, color: st.warn ? "var(--warn-strong)" : "var(--text-muted)" }}>
                      {st.warn && (
                        <svg role="img" aria-label="Warning" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round" style={{ verticalAlign: "-2px", marginRight: 4 }}>
                          <path d="M12 9v4M12 17h.01M10.3 3.9L1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0z" />
                        </svg>
                      )}
                      {st.text}
                    </div>
                  </div>
                  <span style={{ display: "flex", flexDirection: "column", alignItems: "flex-end", gap: 2 }}>
                    {data.has_prices && a.total && (
                      <span className="num" style={{ fontWeight: 500 }} data-testid="account-total" title={`${amount(a.total.amount)} ${a.total.currency}`}>
                        {compact(a.total.amount)} {a.total.currency}
                      </span>
                    )}
                    <span style={{ fontSize: 12, color: "var(--text-muted)" }}>
                      {a.activity_count} activit{a.activity_count === 1 ? "y" : "ies"}
                    </span>
                  </span>
                </div>
              );
            })}
          </div>
        </Card>
        <div style={{ display: "flex", flexDirection: "column", gap: 14 }}>
          <InboxPreview />
          {data.has_prices && <AllocationCard allocation={data.allocation} currency={data.base_currency} />}
        </div>
      </div>
    </>
  );
}

const ALLOCATION_COLORS = ["var(--accent)", "var(--info)", "var(--warn)", "#8a939c", "#5ec2b0", "#b08a3e", "#6b5ca5", "#3a434d"];

function AllocationCard({ allocation, currency }: { allocation: Allocation; currency: string }) {
  const positive = allocation.rows.filter((r) => !r.value.startsWith("-"));
  return (
    <Card title="Allocation" label="Allocation" aside={<span style={{ fontSize: 12, color: "var(--text-muted)" }}>By asset class · {currency}</span>}>
      {allocation.rows.length === 0 ? (
        <p style={{ margin: 0, color: "var(--text-muted)" }}>Nothing is valued yet.</p>
      ) : (
        <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
          <div aria-hidden="true" style={{ display: "flex", height: 10, borderRadius: 999, overflow: "hidden", gap: 2 }}>
            {positive.map((r, i) => (
              <span key={r.class} style={{ flex: Number(r.share) || 0, background: ALLOCATION_COLORS[i % ALLOCATION_COLORS.length] }} />
            ))}
          </div>
          {allocation.rows.map((r) => {
            const i = positive.indexOf(r);
            return (
              <div key={r.class} data-testid="allocation-row" style={{ display: "grid", gridTemplateColumns: "12px minmax(0, 1fr) auto 56px", gap: 10, alignItems: "center", fontSize: 13 }}>
                <span aria-hidden="true" style={{ width: 10, height: 10, borderRadius: 3, background: i >= 0 ? ALLOCATION_COLORS[i % ALLOCATION_COLORS.length] : "transparent", border: i >= 0 ? 0 : "1px solid var(--border-strong)" }} />
                <span>{CLASS_LABELS[r.class] ?? r.class}</span>
                <span className="num" style={{ color: "var(--text-secondary)" }}>
                  {amount(r.value)}
                </span>
                <span className="num" style={{ textAlign: "right" }}>
                  {r.share ? `${r.share}%` : "—"}
                </span>
              </div>
            );
          })}
          {allocation.excluded > 0 && (
            <span style={{ fontSize: 12, color: "var(--text-muted)" }}>
              Leaves out {allocation.excluded} line{allocation.excluded === 1 ? "" : "s"} without a price or exchange rate
            </span>
          )}
        </div>
      )}
    </Card>
  );
}

/** The three newest open Inbox items, the count and a link to the Inbox. */
function InboxPreview() {
  const { data } = useInbox();
  const navigate = useNavigate();
  const n = data?.items.length ?? 0;
  return (
    <Card
      title="Inbox"
      label="Inbox"
      aside={
        n > 0 ? (
          <Link to="/inbox" style={{ fontSize: 13, fontWeight: 500 }}>
            Open Inbox · {n}
          </Link>
        ) : undefined
      }
    >
      {!data ? null : n === 0 ? (
        <p style={{ margin: 0, color: "var(--text-muted)" }}>Everything is in order.</p>
      ) : (
        <ul style={{ listStyle: "none", margin: 0, padding: 0 }}>
          {data.items.slice(0, 3).map((i) => (
            <li key={i.key} style={{ borderTop: "1px solid var(--divider)" }}>
              <button
                type="button"
                data-testid="inbox-preview-item"
                onClick={() => navigate("/inbox", { state: { select: i.key } satisfies InboxSelectRequest })}
                style={{ display: "flex", flexDirection: "column", gap: 2, width: "100%", padding: "10px 0", border: 0, background: "none", font: "inherit", color: "inherit", textAlign: "left", cursor: "pointer" }}
              >
                <span style={{ fontWeight: 500 }}>{i.title}</span>
                {i.subtitle && <span style={{ fontSize: 12, color: "var(--text-muted)" }}>{i.subtitle}</span>}
              </button>
            </li>
          ))}
        </ul>
      )}
    </Card>
  );
}

const small = { fontSize: 12, color: "var(--text-muted)" } as const;

/** Net worth, time- and money-weighted return and income for the scope and period. */
function SummaryCards({ data }: { data: OverviewData }) {
  const { view } = useView();
  const { period, account_id: accountId, real } = view;
  const perf = usePerformance(period, accountId).data;
  const history = useValueHistory(accountId).data;
  const income = useIncome(period, accountId).data;
  const periodLabel = PERIODS.find((p) => p.value === period)?.label ?? "";

  // Change in value over the period, from the daily value history.
  let change: string | null = null;
  const points = history?.points ?? [];
  const last = [...points].reverse().find((p) => p.value !== null);
  if (last && data.has_prices) {
    const from = periodStart(period, points[points.length - 1]!.date);
    const before = from ? [...points].reverse().find((p) => p.date < from) : undefined;
    const startValue = from === null || !before ? 0 : before.value === null ? null : Number(before.value);
    if (startValue !== null) {
      const d = Number(last.value) - startValue;
      change = `${compact(`${d >= 0 ? "+" : "-"}${Math.abs(d).toFixed(2)}`)} ${data.base_currency} ${period === "since_start" ? "since start" : "this period"}`;
    }
  }

  const r = perf?.returns;
  const inflationNote =
    real && perf?.inflation.through ? `inflation through ${perf.inflation.through}${r?.real_provisional ? " · provisional" : ""}` : null;
  const realMissing = real && perf?.inflation.unavailable ? `not available: ${perf.inflation.unavailable}` : null;
  const bench = r?.benchmark ?? null;
  return (
    <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(210px, 1fr))", gap: 14 }}>
      <Card label="Portfolio value">
        <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
          <span style={small}>Net worth</span>
          {data.has_prices ? (
            <>
              <span
                className="num"
                style={{ fontSize: 24, fontWeight: 500, letterSpacing: "-0.02em" }}
                data-testid="portfolio-total"
                title={`${amount(data.total.amount)} ${data.total.currency}`}
              >
                {compact(data.total.amount)} {data.total.currency}
              </span>
              {change && (
                <span className="num" style={{ fontSize: 12.5, color: change.startsWith("−") ? "var(--loss)" : "var(--gain)" }} data-testid="value-change">
                  {change}
                </span>
              )}
              <span style={small}>
                {data.total.oldest_price ? `Prices as of ${longDate(data.total.oldest_price)} or later` : null}
                {excluded(data.total) ? ` · ${excluded(data.total)}` : ""}
              </span>
            </>
          ) : (
            <EmptyState title="Prices needed" message="Refresh prices to see the portfolio value. Assign price sources on the Assets screen." action={{ to: "/assets", label: "Go to Assets" }} />
          )}
        </div>
      </Card>
      <Card label="Time-weighted return">
        <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
          <span style={small}>Time-weighted return · {periodLabel}</span>
          {r && (realMissing ? <span style={small}>{realMissing}</span> : <FigureText figure={real ? r.real_twr : r.twr} size={22} />)}
          {bench && !realMissing && (
            <span style={small} data-testid="benchmark-line">
              Benchmark {bench.name}: {(real ? bench.real : bench.figure).value ?? `not available: ${(real ? bench.real : bench.figure).reason}`}
            </span>
          )}
          {inflationNote && <span style={small}>{inflationNote}</span>}
        </div>
      </Card>
      <Card label="Money-weighted return">
        <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
          <span style={small}>Money-weighted return · {periodLabel}</span>
          {r && (realMissing ? <span style={small}>{realMissing}</span> : <FigureText figure={real ? r.real_mwr : r.mwr} size={22} />)}
          {inflationNote && <span style={small}>{inflationNote}</span>}
        </div>
      </Card>
      <Card label="Income">
        <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
          <span style={small}>Income · {periodLabel}</span>
          {income && (
            <>
              <span className="num" style={{ fontSize: 22, fontWeight: 500 }} data-testid="income-net" title={`${amount(income.totals.net)} ${income.base_currency}`}>
                {compact(income.totals.net)} {income.base_currency}
              </span>
              <span style={small}>
                {compact(income.totals.withheld)} {income.base_currency} tax withheld
                {income.not_converted > 0 ? ` · leaves out ${income.not_converted} without a rate` : ""}
              </span>
            </>
          )}
        </div>
      </Card>
    </div>
  );
}
