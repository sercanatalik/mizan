import { useEffect, useState, type ReactNode } from "react";
import { Link, useLocation, useNavigate, useParams } from "react-router";
import { PageHeader } from "../app/Layout";
import type { TaxTreatment } from "../bridge/api";
import type { AccountDetail } from "../bridge/generated/AccountDetail";
import type { ChartPeriod } from "../bridge/generated/ChartPeriod";
import type { Period } from "../bridge/generated/Period";
import { useAccountChart, useAccountDetail, useAccountMutations, useMergePreview } from "../bridge/queries";
import { Button, Card, DataTable, Donut, EmptyState, LineChart, Segmented, StackedAreaChart, type StackedSeries } from "../ui";
import { CheckText, TREATMENT_LABEL } from "./Accounts";
import { kindLabel } from "./Activity";
import { amount, longDate } from "./format";
import { FigureText } from "./Performance";

const muted = { fontSize: 12, color: "var(--text-muted)" } as const;
const selectStyle = { font: "inherit", fontSize: 13, padding: "4px 8px", borderRadius: 6, border: "1px solid var(--border-strong)" } as const;
const inputStyle = { ...selectStyle, minWidth: 160 } as const;
const right = { textAlign: "right" } as const;

const SECOND_CURRENCIES = ["TRY", "USD", "EUR", "GBP"];

const CHART_PERIODS: { value: ChartPeriod; label: string }[] = [
  { value: "one_month", label: "1M" },
  { value: "three_months", label: "3M" },
  { value: "six_months", label: "6M" },
  { value: "ytd", label: "YTD" },
  { value: "one_year", label: "1Y" },
  { value: "three_years", label: "3Y" },
  { value: "five_years", label: "5Y" },
];

const PERIOD_LABEL: Record<Period, string> = {
  one_month: "1 month",
  three_months: "3 months",
  ytd: "Year to date",
  last12_months: "1 year",
  since_start: "Since start",
};

/** The chart shows at most eight bands; the smallest positions fold into "Other". */
const MAX_BANDS = 8;

const compact = (v: number) =>
  Math.abs(v) >= 10_000 ? `${Math.round(v / 1000).toLocaleString("en-US")}K` : Math.round(v).toLocaleString("en-US");

const num = (v: string | null): number | null => (v === null ? null : Number(v));
const lossColor = (v: string | null | undefined) => (v && (v.startsWith("-") || v.startsWith("−")) ? "var(--loss)" : undefined);

function Tile({ label, value, sub }: { label: string; value: ReactNode; sub?: ReactNode }) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 4, padding: 14, border: "1px solid var(--border)", borderRadius: 10, background: "var(--surface)" }} data-testid="tile">
      <span style={muted}>{label}</span>
      <span className="num" style={{ fontSize: 22, fontWeight: 500 }}>
        {value}
      </span>
      {sub && <span style={muted}>{sub}</span>}
    </div>
  );
}

function Rename({ d }: { d: AccountDetail }) {
  const { rename } = useAccountMutations();
  const [name, setName] = useState<string | null>(null);
  if (name === null) {
    return (
      <Button small variant="ghost" onClick={() => setName(d.name)}>
        Rename
      </Button>
    );
  }
  const save = () => rename.mutate({ id: d.id, name }, { onSuccess: () => setName(null) });
  return (
    <form
      aria-label="Rename account"
      style={{ display: "inline-flex", gap: 6, alignItems: "center" }}
      onSubmit={(e) => {
        e.preventDefault();
        save();
      }}
    >
      <input aria-label="Account name" value={name} onChange={(e) => setName(e.target.value)} style={inputStyle} autoFocus />
      <Button small variant="primary" type="submit" disabled={rename.isPending || name.trim() === ""}>
        Save
      </Button>
      <Button small variant="ghost" onClick={() => setName(null)}>
        Cancel
      </Button>
      {rename.error && (
        <span role="alert" style={{ fontSize: 12, color: "var(--loss)" }}>
          {String(rename.error)}
        </span>
      )}
    </form>
  );
}

function Identifiers({ d }: { d: AccountDetail }) {
  const { addIdentifier, moveIdentifier } = useAccountMutations();
  const [hint, setHint] = useState("");
  const error = addIdentifier.error ?? moveIdentifier.error;
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
      <span style={muted}>Statement identifiers · used to match future imports to this account</span>
      <ul style={{ listStyle: "none", margin: 0, padding: 0, display: "flex", flexDirection: "column", gap: 6 }}>
        {d.identifiers.length === 0 && <li style={muted}>None</li>}
        {d.identifiers.map((h) => (
          <li key={h} data-testid="identifier" style={{ display: "flex", gap: 8, alignItems: "center", fontSize: 13 }}>
            <code>{h}</code>
            {d.merge_targets.length > 0 && (
              <select
                aria-label={`Move ${h} to`}
                value=""
                style={selectStyle}
                disabled={moveIdentifier.isPending}
                onChange={(e) => e.target.value && moveIdentifier.mutate({ identifier: h, to: Number(e.target.value) })}
              >
                <option value="">Move to…</option>
                {d.merge_targets.map((t) => (
                  <option key={t.id} value={t.id}>
                    {t.name}
                  </option>
                ))}
              </select>
            )}
          </li>
        ))}
      </ul>
      <form
        aria-label="Add identifier"
        style={{ display: "flex", gap: 6 }}
        onSubmit={(e) => {
          e.preventDefault();
          addIdentifier.mutate({ id: d.id, identifier: hint.trim() }, { onSuccess: () => setHint("") });
        }}
      >
        <input aria-label="New identifier" placeholder="e.g. U1234567" value={hint} onChange={(e) => setHint(e.target.value)} style={inputStyle} />
        <Button small type="submit" disabled={hint.trim() === "" || addIdentifier.isPending}>
          Add
        </Button>
      </form>
      {error && (
        <span role="alert" style={{ fontSize: 12, color: "var(--loss)" }}>
          {String(error)}
        </span>
      )}
    </div>
  );
}

function Merge({ d }: { d: AccountDetail }) {
  const { merge } = useAccountMutations();
  const navigate = useNavigate();
  const [open, setOpen] = useState(false);
  const [into, setInto] = useState("");
  const preview = useMergePreview(d.id, open && into !== "");
  if (d.merge_targets.length === 0) return null;
  if (!open) {
    return (
      <Button small onClick={() => setOpen(true)}>
        Merge…
      </Button>
    );
  }
  const target = d.merge_targets.find((t) => String(t.id) === into);
  const p = preview.data;
  return (
    <div role="group" aria-label="Merge account" style={{ display: "flex", flexDirection: "column", gap: 8, padding: 12, border: "1px solid var(--border)", borderRadius: 8 }}>
      <label style={{ display: "flex", gap: 8, alignItems: "center", fontSize: 13 }}>
        Merge {d.name} into
        <select aria-label="Merge into" value={into} onChange={(e) => setInto(e.target.value)} style={selectStyle}>
          <option value="">Choose an account</option>
          {d.merge_targets.map((t) => (
            <option key={t.id} value={t.id}>
              {t.name}
            </option>
          ))}
        </select>
      </label>
      {target && p && (
        <p style={{ margin: 0, fontSize: 13 }} data-testid="merge-preview">
          {p.activities} activities, {p.balance_checks} balance checks and {p.identifiers} identifiers will move to {target.name}. Transfers between the two accounts become internal. You can undo
          the merge on the Accounts screen.
        </p>
      )}
      {merge.error && (
        <span role="alert" style={{ fontSize: 12, color: "var(--loss)" }}>
          {String(merge.error)}
        </span>
      )}
      <div style={{ display: "flex", gap: 6 }}>
        <Button
          small
          variant="primary"
          disabled={!target || !p || merge.isPending}
          onClick={() => target && merge.mutate({ from: d.id, into: target.id }, { onSuccess: () => void navigate(`/accounts/${target.id}`) })}
        >
          Merge
        </Button>
        <Button small variant="ghost" onClick={() => setOpen(false)}>
          Cancel
        </Button>
      </div>
    </div>
  );
}

function Delete({ d }: { d: AccountDetail }) {
  const { remove } = useAccountMutations();
  const navigate = useNavigate();
  const [confirming, setConfirming] = useState(false);
  if (d.delete_blocker) {
    return (
      <span style={muted} data-testid="delete-blocker">
        Can’t delete: {d.delete_blocker}
      </span>
    );
  }
  if (!confirming) {
    return (
      <Button small variant="ghost" onClick={() => setConfirming(true)}>
        Delete…
      </Button>
    );
  }
  return (
    <span style={{ display: "inline-flex", gap: 6, alignItems: "center", fontSize: 13 }}>
      Delete {d.name}?
      <Button small variant="primary" disabled={remove.isPending} onClick={() => remove.mutate(d.id, { onSuccess: () => void navigate("/accounts") })}>
        Delete account
      </Button>
      <Button small variant="ghost" onClick={() => setConfirming(false)}>
        Cancel
      </Button>
      {remove.error && (
        <span role="alert" style={{ fontSize: 12, color: "var(--loss)" }}>
          {String(remove.error)}
        </span>
      )}
    </span>
  );
}

const PROVIDERS: { value: string; label: string }[] = [
  { value: "", label: "Per asset" },
  { value: "tefas", label: "TEFAS" },
  { value: "yahoo", label: "Yahoo Finance" },
  { value: "twelvedata", label: "Twelve Data" },
];

function PriceSource({ d }: { d: AccountDetail }) {
  const { priceSource } = useAccountMutations();
  const out = priceSource.data;
  return (
    <span style={{ display: "inline-flex", flexDirection: "column", gap: 4 }}>
      <label style={{ display: "inline-flex", gap: 8, alignItems: "center" }}>
        Price source
        <select
          aria-label="Price source"
          value={d.price_source ?? ""}
          style={selectStyle}
          disabled={priceSource.isPending}
          onChange={(e) => priceSource.mutate({ id: d.id, provider: e.target.value || null })}
        >
          {PROVIDERS.map((p) => (
            <option key={p.value} value={p.value}>
              {p.label}
            </option>
          ))}
        </select>
      </label>
      <span style={muted}>
        {d.price_source ? "Assets in this account without a price source get it from their symbol, now and on future imports. Sources set on the Assets screen are kept." : "Each asset's price source is set on the Assets screen."}
      </span>
      {out && (
        <span role="status" style={muted} data-testid="price-source-outcome">
          {out.set === 0 ? "No assets needed a price source." : `Price source set for ${out.set} asset${out.set === 1 ? "" : "s"}; refresh prices to fetch them.`}
          {out.skipped.length > 0 && ` Left as they were (no usable symbol): ${out.skipped.join(", ")}.`}
        </span>
      )}
      {priceSource.error && (
        <span role="alert" style={{ fontSize: 12, color: "var(--loss)" }}>
          {String(priceSource.error)}
        </span>
      )}
    </span>
  );
}

function Settings({ d }: { d: AccountDetail }) {
  const { setTreatment, secondCurrency } = useAccountMutations();
  const base = d.summary.base_currency;
  return (
    <div style={{ display: "flex", gap: 16, flexWrap: "wrap", alignItems: "flex-start", fontSize: 13 }}>
      <PriceSource d={d} />
      <label style={{ display: "inline-flex", gap: 8, alignItems: "center" }}>
        Tax treatment
        <select
          aria-label="Tax treatment"
          value={d.tax_treatment}
          style={selectStyle}
          disabled={setTreatment.isPending}
          onChange={(e) => setTreatment.mutate({ id: d.id, treatment: e.target.value as TaxTreatment })}
        >
          {Object.entries(TREATMENT_LABEL).map(([v, l]) => (
            <option key={v} value={v}>
              {l}
            </option>
          ))}
        </select>
      </label>
      <label style={{ display: "inline-flex", gap: 8, alignItems: "center" }}>
        Second currency
        <select aria-label="Second currency" value={d.summary.second_currency} style={selectStyle} onChange={(e) => secondCurrency.mutate(e.target.value)}>
          {SECOND_CURRENCIES.filter((c) => c !== base).map((c) => (
            <option key={c}>{c}</option>
          ))}
        </select>
      </label>
    </div>
  );
}

function Tiles({ d }: { d: AccountDetail }) {
  const s = d.summary;
  const base = s.base_currency;
  const cash = Number(s.cash_value);
  return (
    <section aria-label="Account summary" style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(min(200px, 100%), 1fr))", gap: 12 }}>
      <Tile label={`Value · ${base}`} value={<span style={{ color: lossColor(s.value) }}>{amount(s.value)}</span>} sub={s.cost ? `Cost ${amount(s.cost)}${s.unrealized ? ` · gain ${amount(s.unrealized)}` : ""}` : undefined} />
      <Tile label={`Value · ${s.second_currency}`} value={s.value_second ? amount(s.value_second) : "–"} sub={s.rate_note ?? "No rate for this currency"} />
      <Tile
        label="Last day"
        value={s.last_day ? <span style={{ color: lossColor(s.last_day.amount) }}>{amount(s.last_day.amount)}</span> : "–"}
        sub={s.last_day ? `${s.last_day.pct} · to ${longDate(s.last_day.date)}` : "Needs two price dates"}
      />
      <Tile
        label="1-year return"
        value={<FigureText figure={s.one_year} size={22} />}
        sub={
          <>
            {s.one_year_second && `${s.one_year_second} in ${s.second_currency} · `}
            YTD {s.ytd.value ?? "–"}
          </>
        }
      />
      <Tile
        label="Positions and cash"
        value={amount(s.positions_value)}
        sub={
          <span>
            positions · cash <span style={{ color: cash < 0 ? "var(--loss)" : undefined }}>{amount(s.cash_value)}</span>
            {s.cash.length > 0 && ` (${s.cash.map((c) => `${amount(c.amount)} ${c.currency}`).join(", ")})`}
          </span>
        }
      />
    </section>
  );
}

/** Folds the bands past the eighth into "Other", smallest weights last. */
export function foldBands(bands: { name: string; values: (number | null)[]; weight: number }[]): StackedSeries[] {
  const sorted = [...bands].sort((a, b) => b.weight - a.weight);
  if (sorted.length <= MAX_BANDS) return sorted.map(({ name, values }) => ({ name, values }));
  const keep = sorted.slice(0, MAX_BANDS - 1);
  const rest = sorted.slice(MAX_BANDS - 1);
  const other = rest[0]!.values.map((_, i) => {
    const vs = rest.map((b) => b.values[i]).filter((v): v is number => v !== null && v !== undefined);
    return vs.length ? vs.reduce((a, v) => a + v, 0) : null;
  });
  return [...keep.map(({ name, values }) => ({ name, values })), { name: `Other (${rest.length})`, values: other }];
}

function CurrentUnits({ d }: { d: AccountDetail }) {
  const base = d.summary.base_currency;
  const [period, setPeriod] = useState<ChartPeriod>("one_year");
  const [currency, setCurrency] = useState(base);
  const { data: chart, error } = useAccountChart(d.id, period, currency);
  const currencies = [base, d.summary.second_currency].filter((c, i, a) => a.indexOf(c) === i);
  const slices = d.holdings.filter((h) => h.value !== null && Number(h.value) > 0).map((h) => ({ name: h.symbol ?? h.name ?? h.key, value: Number(h.value), label: h.weight ?? undefined }));
  const series = chart
    ? foldBands(chart.bands.map((b, i) => ({ name: b.name, values: b.values.map(num), weight: Number((chart.weights[i] ?? "0").replace("%", "")) })))
    : [];
  const short = chart?.bands.filter((b) => b.short_history).map((b) => b.name) ?? [];
  return (
    <div style={{ display: "grid", gridTemplateColumns: "minmax(0, 3fr) minmax(200px, 1fr)", gap: 14 }}>
      <Card
        title="Value at current units"
        label="Value at current units"
        aside={
          <span style={{ display: "inline-flex", gap: 8, flexWrap: "wrap" }}>
            <Segmented label="Chart period" options={CHART_PERIODS} value={period} onChange={setPeriod} />
            {currencies.length > 1 && <Segmented label="Chart currency" options={currencies.map((c) => ({ value: c, label: c }))} value={currency} onChange={setCurrency} />}
          </span>
        }
      >
        <span style={muted}>Today’s units at each day’s prices · shows price effect only, not deposits or trades</span>
        {error ? (
          <EmptyState title="Chart not available" message={String(error)} />
        ) : !chart || chart.dates.length === 0 || series.length === 0 ? (
          <EmptyState title="No price history yet" message="The chart appears once prices have been downloaded for this account’s positions." />
        ) : (
          <>
            <StackedAreaChart label="Value at current units" dates={chart.dates} series={series} format={compact} formatDate={longDate} />
            {short.length > 0 && (
              <span role="note" style={muted}>
                Prices start later than the period for {short.join(", ")}.
              </span>
            )}
          </>
        )}
      </Card>
      <Card title="Where the money sits" label="Allocation">
        {slices.length === 0 ? <span style={muted}>No priced positions</span> : <Donut label="Position weights" slices={slices} />}
      </Card>
    </div>
  );
}

function Holdings({ d }: { d: AccountDetail }) {
  const s = d.summary;
  const pct = (v: string | null) => <span style={{ color: lossColor(v) }}>{v ?? "–"}</span>;
  return (
    <Card title="Holdings" label="Account holdings">
      {d.holdings.length === 0 ? (
        <span style={muted}>No open positions</span>
      ) : (
        <DataTable label="Account holdings" minWidth={900}>
          <thead>
            <tr>
              <th scope="col">Position</th>
              <th scope="col" style={right}>Units</th>
              <th scope="col" style={right}>Price</th>
              <th scope="col" style={right}>Value · {s.base_currency}</th>
              <th scope="col" style={right}>Value · {s.second_currency}</th>
              <th scope="col" style={right}>Weight</th>
              <th scope="col" style={right}>Day</th>
              <th scope="col" style={right}>1M</th>
              <th scope="col" style={right}>1Y</th>
            </tr>
          </thead>
          <tbody>
            {d.holdings.map((h) => (
              <tr key={h.key} data-testid="holding-row">
                <td>
                  <div style={{ fontWeight: 500 }}>{h.symbol ?? h.key}</div>
                  {h.name && <div style={muted}>{h.name}</div>}
                  {h.note && <div style={{ fontSize: 12, color: "var(--warn-strong)" }}>{h.note}</div>}
                </td>
                <td className="num" style={{ ...right, color: lossColor(h.units) }}>
                  {amount(h.units)}
                </td>
                <td className="num" style={right}>
                  {h.price ? `${amount(h.price)} ${h.price_currency ?? ""}` : "–"}
                  {h.price_date && <div style={muted}>{longDate(h.price_date)}</div>}
                </td>
                <td className="num" style={{ ...right, color: lossColor(h.value) }}>
                  {h.value ? amount(h.value) : "–"}
                </td>
                <td className="num" style={right}>
                  {h.value_second ? amount(h.value_second) : "–"}
                </td>
                <td className="num" style={right}>
                  {h.weight ?? "–"}
                </td>
                <td className="num" style={right}>
                  {pct(h.day)}
                </td>
                <td className="num" style={right}>
                  {pct(h.month)}
                </td>
                <td className="num" style={right}>
                  {pct(h.year)}
                </td>
              </tr>
            ))}
            <tr data-testid="positions-total-row" style={{ fontWeight: 500 }}>
              <td>Positions total</td>
              <td colSpan={2} />
              <td className="num" style={{ ...right, color: lossColor(d.total_value) }}>
                {amount(d.total_value)}
              </td>
              <td className="num" style={right}>
                {d.total_value_second ? amount(d.total_value_second) : "–"}
              </td>
              <td colSpan={4} />
            </tr>
            <tr data-testid="cash-row">
              <td>Cash</td>
              <td colSpan={2} style={muted}>
                {s.cash.map((c) => `${amount(c.amount)} ${c.currency}`).join(", ")}
              </td>
              <td className="num" style={{ ...right, color: lossColor(s.cash_value) }}>
                {amount(s.cash_value)}
              </td>
              <td colSpan={5} />
            </tr>
            <tr data-testid="total-row" style={{ fontWeight: 500 }}>
              <td>Account value</td>
              <td colSpan={2} />
              <td className="num" style={{ ...right, color: lossColor(s.value) }}>
                {amount(s.value)}
              </td>
              <td className="num" style={right}>
                {s.value_second ? amount(s.value_second) : "–"}
              </td>
              <td colSpan={4} />
            </tr>
          </tbody>
        </DataTable>
      )}
    </Card>
  );
}

function History({ d }: { d: AccountDetail }) {
  const base = d.summary.base_currency;
  const pts = d.history;
  return (
    <Card title="Value over time" label="Account value chart" aside={<span style={muted}>Actual units · {base}</span>}>
      {pts.length === 0 || pts.every((p) => p.value === null) ? (
        <span style={muted}>No value history yet; it appears once price history has been downloaded.</span>
      ) : (
        <LineChart
          label="Account value and net contributions"
          dates={pts.map((p) => p.date)}
          series={[
            { name: "Value", values: pts.map((p) => num(p.value)), labels: pts.map((p) => (p.value === null ? null : `${p.value} ${base}`)) },
            { name: "Net contributions", values: pts.map((p) => Number(p.contributions)), labels: pts.map((p) => `${p.contributions} ${base}`) },
          ]}
          format={compact}
          formatDate={longDate}
        />
      )}
    </Card>
  );
}

function Returns({ d }: { d: AccountDetail }) {
  return (
    <Card title="Returns" label="Account returns">
      <DataTable label="Account returns" minWidth={520}>
        <thead>
          <tr>
            <th scope="col">Period</th>
            <th scope="col" style={right}>Time-weighted</th>
            <th scope="col" style={right}>Money-weighted</th>
          </tr>
        </thead>
        <tbody>
          {d.returns.map((r) => (
            <tr key={r.period} data-testid="period-row">
              <td>
                {PERIOD_LABEL[r.period]}
                <div style={muted}>from {longDate(r.from)}</div>
              </td>
              <td style={right}>
                <FigureText figure={r.twr} />
              </td>
              <td style={right}>
                <FigureText figure={r.mwr} />
              </td>
            </tr>
          ))}
        </tbody>
      </DataTable>
    </Card>
  );
}

function Income({ d }: { d: AccountDetail }) {
  const base = d.summary.base_currency;
  return (
    <Card title="Income" label="Account income" aside={<span style={muted}>Dividends, distributions and interest · {base}</span>}>
      {d.income.length === 0 ? (
        <span style={muted}>No income recorded</span>
      ) : (
        <DataTable label="Income by year" minWidth={360}>
          <thead>
            <tr>
              <th scope="col">Year</th>
              <th scope="col" style={right}>Gross</th>
              <th scope="col" style={right}>Tax withheld</th>
            </tr>
          </thead>
          <tbody>
            {d.income.map((y) => (
              <tr key={y.year} data-testid="income-row">
                <td>{y.year}</td>
                <td className="num" style={right}>
                  {amount(y.gross)}
                </td>
                <td className="num" style={right}>
                  {amount(y.withheld)}
                </td>
              </tr>
            ))}
          </tbody>
        </DataTable>
      )}
    </Card>
  );
}

function Recent({ d }: { d: AccountDetail }) {
  return (
    <Card title="Recent activity" label="Recent activity" aside={<Link to={`/activity?account=${d.id}`}>All activity</Link>}>
      {d.recent.length === 0 ? (
        <span style={muted}>No activities</span>
      ) : (
        <DataTable label="Recent activity" minWidth={520}>
          <thead>
            <tr>
              <th scope="col">Date</th>
              <th scope="col">Kind</th>
              <th scope="col">Instrument</th>
              <th scope="col" style={right}>Quantity</th>
              <th scope="col" style={right}>Cash</th>
            </tr>
          </thead>
          <tbody>
            {d.recent.map((r) => (
              <tr key={r.id} data-testid="recent-row">
                <td>{longDate(r.date)}</td>
                <td>{kindLabel(r.kind)}</td>
                <td>{r.symbol ?? r.instrument ?? "–"}</td>
                <td className="num" style={right}>
                  {r.quantity ? amount(r.quantity) : "–"}
                </td>
                <td className="num" style={{ ...right, color: lossColor(r.net_cash?.amount) }}>
                  {r.net_cash ? `${amount(r.net_cash.amount)} ${r.net_cash.currency}` : "–"}
                </td>
              </tr>
            ))}
          </tbody>
        </DataTable>
      )}
    </Card>
  );
}

function Checks({ d }: { d: AccountDetail }) {
  return (
    <Card title="Balance checks" label="Balance checks" id="balance-checks">
      {d.checks.length === 0 ? (
        <span style={muted}>No balance checks. Statements with closing balances add them on import.</span>
      ) : (
        <ul style={{ listStyle: "none", margin: 0, padding: 0, display: "flex", flexDirection: "column", gap: 6 }}>
          {d.checks.map((c, i) => (
            <li key={`${c.as_of}-${i}`} data-testid="check-row">
              <CheckText check={c} />
            </li>
          ))}
        </ul>
      )}
    </Card>
  );
}

export function Account() {
  const { id } = useParams();
  const accountId = Number(id);
  const { data: d, error } = useAccountDetail(accountId);
  // The Inbox links to /accounts/<id>#balance-checks.
  const { hash } = useLocation();
  const loaded = d !== undefined;
  useEffect(() => {
    if (loaded && hash) document.getElementById(hash.slice(1))?.scrollIntoView?.({ block: "start" });
  }, [loaded, hash]);
  if (error) {
    return (
      <>
        <PageHeader title="Account" />
        <EmptyState title="Account not found" message="It may have been merged or deleted." action={{ to: "/accounts", label: "Go to Accounts" }} />
      </>
    );
  }
  if (!d) return <PageHeader title="Account" />;
  return (
    <>
      <PageHeader
        title={d.name}
        subtitle={
          <span style={{ display: "inline-flex", gap: 10, alignItems: "center" }}>
            <Link to="/accounts">Accounts</Link>
            <span>{TREATMENT_LABEL[d.tax_treatment] ?? d.tax_treatment}</span>
          </span>
        }
        actions={
          <span style={{ display: "inline-flex", gap: 8, alignItems: "flex-start", flexWrap: "wrap" }}>
            <Rename d={d} />
            <Merge d={d} />
            <Delete d={d} />
          </span>
        }
      />
      <Card title="Details" label="Account details">
        <Settings d={d} />
        <Identifiers d={d} />
      </Card>
      <Tiles d={d} />
      {d.summary.notes.length > 0 && (
        <p role="note" style={{ ...muted, margin: 0 }}>
          {d.summary.notes.join(" · ")}
        </p>
      )}
      <CurrentUnits d={d} />
      <Holdings d={d} />
      <History d={d} />
      <Returns d={d} />
      <Income d={d} />
      <Recent d={d} />
      <Checks d={d} />
    </>
  );
}
