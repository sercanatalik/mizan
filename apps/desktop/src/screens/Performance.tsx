import { Link } from "react-router";
import { PageHeader } from "../app/Layout";
import { useScopeName, useView } from "../app/view";
import type { Figure } from "../bridge/generated/Figure";
import type { PerformanceView } from "../bridge/generated/PerformanceView";
import { useContributions, usePerformance, useSetBenchmark } from "../bridge/queries";
import { Card, DataTable, EmptyState, LineChart } from "../ui";
import { longDate } from "./format";
import { NoData } from "./NoData";

type Mode = "nominal" | "real";

const muted = { fontSize: 12, color: "var(--text-muted)" } as const;

function estimateText(f: Figure): string | null {
  return f.estimated_days > 0 ? `estimate · ${f.estimated_days} day${f.estimated_days === 1 ? "" : "s"} at trade price or cost` : null;
}

/** A return figure, or "not available" with the reason; with the estimate mark when it applies. */
export function FigureText({ figure, size = 13 }: { figure: Figure; size?: number }) {
  if (!figure.value) {
    return (
      <span style={{ fontSize: Math.min(size, 13), color: "var(--text-muted)" }} data-testid="figure-na">
        not available: {figure.reason}
      </span>
    );
  }
  const est = estimateText(figure);
  return (
    <span style={{ display: "inline-flex", flexDirection: "column", gap: 2 }}>
      <span className="num" style={{ fontSize: size, fontWeight: 500, color: figure.value.startsWith("−") ? "var(--loss)" : "var(--text)" }}>
        {figure.value}
        {figure.annualized && <span style={{ ...muted, fontWeight: 400, marginLeft: 6 }}>{figure.annualized} a year</span>}
      </span>
      {est && (
        <span style={{ fontSize: 12, color: "var(--warn-strong)" }} data-testid="estimate">
          {est}
        </span>
      )}
    </span>
  );
}

function sinceLabel(from: string | null, clamped: boolean, to: string): string {
  if (!from) return "";
  return clamped ? `since ${longDate(from)}` : `${longDate(from)} – ${longDate(to)}`;
}

function BenchmarkPicker({ view }: { view: PerformanceView }) {
  const set = useSetBenchmark();
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 4, minWidth: 0, width: "100%" }}>
      <label style={{ display: "flex", flexWrap: "wrap", alignItems: "center", gap: 8, fontSize: 13, minWidth: 0 }}>
        Benchmark
        <select
          value={view.benchmark_asset_id ?? ""}
          onChange={(e) => set.mutate(e.target.value === "" ? null : Number(e.target.value))}
          style={{
            font: "inherit",
            padding: "4px 8px",
            borderRadius: 6,
            border: "1px solid var(--border-strong)",
            flex: "1 1 140px",
            minWidth: 0,
            maxWidth: "100%",
            textOverflow: "ellipsis",
          }}
        >
          <option value="">None</option>
          {view.benchmark_options.map((o) => (
            <option key={o.asset_id} value={o.asset_id}>
              {o.name}
            </option>
          ))}
        </select>
      </label>
      <Link to="/assets" style={{ fontSize: 12 }}>
        Choose benchmarks on Assets
      </Link>
    </div>
  );
}

function Returns({ view, mode }: { view: PerformanceView; mode: Mode }) {
  const r = view.returns;
  const bench = r.benchmark;
  const real = mode === "real";
  const excludedNote = r.excluded.length ? `Excludes ${r.excluded.join("; ")}` : null;
  if (real && view.inflation.unavailable) {
    return (
      <Card title="Real returns" label="Real returns">
        <EmptyState title="Real returns are not available" message={`Real returns are not available: ${view.inflation.unavailable}.`} />
      </Card>
    );
  }
  const twr = real ? r.real_twr : r.twr;
  const mwr = real ? r.real_mwr : r.mwr;
  const benchFigure = bench && (real ? bench.real : bench.figure);
  const portfolioPoint = (g: (typeof r.growth)[number]) => (real ? g.portfolio_real : g.portfolio);
  const benchPoint = (g: (typeof r.growth)[number]) => (real ? g.benchmark_real : g.benchmark);
  const inflationNote = real && view.inflation.through ? `After inflation · ${view.inflation.label} through ${view.inflation.through}${r.real_provisional ? " · provisional" : ""}` : null;
  return (
    <>
      {inflationNote && (
        <p style={{ ...muted, margin: 0 }} data-testid="inflation-note">
          {inflationNote}
        </p>
      )}
      <section aria-label="Returns" style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(min(220px, 100%), 1fr))", gap: 14 }}>
        <Card title="Time-weighted return" label="Time-weighted return">
          <FigureText figure={twr} size={26} />
          <span style={muted}>How the investments did, independent of deposits and withdrawals</span>
          {excludedNote && <span style={{ fontSize: 12, color: "var(--warn-strong)" }}>{excludedNote}</span>}
        </Card>
        <Card title="Money-weighted return" label="Money-weighted return">
          <FigureText figure={mwr} size={26} />
          <span style={muted}>Your return, including the timing of your deposits and withdrawals</span>
        </Card>
        <Card title="Benchmark" label="Benchmark" aside={<BenchmarkPicker view={view} />}>
          {bench && benchFigure ? (
            <>
              <FigureText figure={benchFigure} size={26} />
              <span style={muted}>{bench.name}</span>
            </>
          ) : (
            <span style={muted}>Choose an asset with prices, for example a TEFAS fund, to compare against.</span>
          )}
        </Card>
      </section>
      <Card title="Growth" label="Growth chart" aside={<span style={muted}>Time-weighted{real ? ", after inflation" : ""}, 100 at the start · {view.base_currency}</span>}>
        {r.growth.length === 0 ? (
          <EmptyState title="Nothing to show" message={twr.reason ? `Not available: ${twr.reason}.` : "No value history for this period."} />
        ) : (
          <LineChart
            label="Growth"
            dates={r.growth.map((g) => g.date)}
            series={[
              { name: "Portfolio", values: r.growth.map((g) => { const v = portfolioPoint(g); return v === null ? null : Number(v); }), labels: r.growth.map(portfolioPoint) },
              ...(bench && r.growth.some((g) => benchPoint(g) !== null)
                ? [{ name: bench.name, values: r.growth.map((g) => { const v = benchPoint(g); return v === null ? null : Number(v); }), labels: r.growth.map(benchPoint) }]
                : []),
            ]}
            format={(v) => v.toFixed(0)}
            formatDate={longDate}
          />
        )}
      </Card>
      <Card title="Accounts" label="Account returns">
        <DataTable label="Account returns" minWidth={560}>
          <thead>
            <tr>
              <th scope="col">Account</th>
              <th scope="col">Period</th>
              <th scope="col" style={{ textAlign: "right" }}>
                Time-weighted
              </th>
              <th scope="col" style={{ textAlign: "right" }}>
                Money-weighted
              </th>
            </tr>
          </thead>
          <tbody>
            {r.accounts.map((a) => (
              <tr key={a.account_id} data-testid="return-row">
                <td style={{ fontWeight: 500 }}>{a.name}</td>
                <td style={{ ...muted, fontSize: 13 }}>{sinceLabel(a.from, a.clamped, r.to)}</td>
                <td style={{ textAlign: "right" }}>
                  <FigureText figure={real ? a.real_twr : a.twr} />
                </td>
                <td style={{ textAlign: "right" }}>
                  <FigureText figure={real ? a.real_mwr : a.mwr} />
                </td>
              </tr>
            ))}
          </tbody>
        </DataTable>
      </Card>
    </>
  );
}

const PERFORMANCE_VIEW = { scope: true, period: true, currency: true, real: true } as const;

export function Performance() {
  const { view } = useView();
  const scopeName = useScopeName();
  const { period, account_id: accountId } = view;
  const mode: Mode = view.real ? "real" : "nominal";
  const { data } = useContributions(accountId);
  const { data: perf } = usePerformance(period, accountId);
  if (!data) return <PageHeader title="Performance" view={PERFORMANCE_VIEW} />;
  const current = data.periods.find((p) => p.period === period);
  if ((data.periods[0]?.accounts.length ?? 0) === 0 || !current) {
    return (
      <>
        <PageHeader title="Performance" view={PERFORMANCE_VIEW} />
        <NoData what="contributions" />
      </>
    );
  }
  const range = perf ? sinceLabel(perf.returns.from, perf.returns.clamped, perf.returns.to) : `to ${longDate(current.to)}`;
  return (
    <>
      <PageHeader
        title="Performance"
        subtitle={`${scopeName} · ${range}`}
        view={PERFORMANCE_VIEW}
      />
      {perf && <Returns view={perf} mode={mode} />}
      <Card title="Net contributions" label="Net contributions" aside={<span style={muted}>Deposits minus withdrawals; internal transfers excluded</span>}>
        <DataTable label="Net contributions" minWidth={520}>
          <thead>
            <tr>
              <th scope="col">Account</th>
              <th scope="col">From</th>
              <th scope="col" style={{ textAlign: "right" }}>
                Net contribution
              </th>
            </tr>
          </thead>
          <tbody>
            {current.accounts.map((a) => (
              <tr key={a.account_id} data-testid="contribution-row">
                <td style={{ fontWeight: 500 }}>{a.name}</td>
                <td className="num">{a.from ?? "—"}</td>
                <td className="num" style={{ textAlign: "right" }}>
                  {a.by_currency.length ? a.by_currency.map((c) => `${c.amount} ${c.currency}`).join(" · ") : "—"}
                </td>
              </tr>
            ))}
            <tr data-testid="contribution-total">
              <td style={{ fontWeight: 600 }}>Total</td>
              <td />
              <td className="num" style={{ textAlign: "right", fontWeight: 600 }}>
                {current.total.length ? current.total.map((c) => `${c.amount} ${c.currency}`).join(" · ") : "—"}
              </td>
            </tr>
          </tbody>
        </DataTable>
      </Card>
    </>
  );
}
