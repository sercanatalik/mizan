import { open } from "@tauri-apps/plugin-dialog";
import { useState } from "react";
import { useSearchParams } from "react-router";
import { ProvidersTab } from "./market/ProvidersTab";
import { PageHeader } from "../app/Layout";
import { errorMessage, type SeriesKindName } from "../bridge/api";
import type { SeriesDetail } from "../bridge/generated/SeriesDetail";
import type { SeriesFilter } from "../bridge/generated/SeriesFilter";
import type { SeriesPoint } from "../bridge/generated/SeriesPoint";
import type { SeriesRow } from "../bridge/generated/SeriesRow";
import { useMarketDetail, useMarketMutations, useMarketSeries } from "../bridge/queries";
import { Badge, Button, Card, DataTable, EmptyState, LineChart, Segmented } from "../ui";
import s from "./activity.module.css";
import { longDate } from "./format";

type Tab = SeriesKindName | "providers";

const TABS: { value: Tab; label: string }[] = [
  { value: "price", label: "Prices" },
  { value: "rate", label: "Rates" },
  { value: "index", label: "Indices" },
  { value: "providers", label: "Providers" },
];

const EMPTY_FILTER: SeriesFilter = { source: null, currency: null, text: null, used: false, edited: false, attention: false };

const muted = { fontSize: 12, color: "var(--text-muted)" } as const;
const right = { textAlign: "right" } as const;

/** A date or a `YYYY-MM` month for display. */
function period(p: string | null): string {
  if (!p) return "—";
  if (p.length === 7) {
    const [y, m] = p.split("-");
    return longDate(`${y}-${m}-01`).replace(/^1 /, "");
  }
  return longDate(p);
}

function sourceLabel(src: string): string {
  return { tefas: "TEFAS", yahoo: "Yahoo Finance", twelvedata: "Twelve Data", tcmb: "TCMB", eurostat: "Eurostat", bls: "BLS", ons: "ONS", evds: "EVDS", manual: "Manual" }[src] ?? src;
}

// ------------------------------------------------------------------ list

function Toggle({ label, on, set }: { label: string; on: boolean; set: (v: boolean) => void }) {
  return (
    <Button small variant={on ? "primary" : "secondary"} aria-pressed={on} onClick={() => set(!on)}>
      {label}
    </Button>
  );
}

function SeriesTable({ rows, kind, onOpen }: { rows: SeriesRow[]; kind: SeriesKindName; onOpen: (key: string) => void }) {
  return (
    <DataTable label="Series" minWidth={860}>
      <thead>
        <tr>
          <th scope="col">Series</th>
          <th scope="col">Source</th>
          {kind !== "index" && <th scope="col">Currency</th>}
          <th scope="col" style={right}>Latest</th>
          <th scope="col">As of</th>
          <th scope="col">From</th>
          <th scope="col" style={right}>Values</th>
          {kind !== "index" && (
            <th scope="col" style={right} title="Weekdays you held it without a value (holidays included)">
              Gaps while held
            </th>
          )}
          <th scope="col">Quality</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((r) => (
          <tr key={r.key} data-testid="series-row">
            <td>
              <button
                type="button"
                onClick={() => onOpen(r.key)}
                aria-label={`Open ${r.name}`}
                style={{ all: "unset", cursor: "pointer", fontWeight: 500, textDecoration: "underline dotted" }}
              >
                {r.name}
              </button>
              {r.code && r.code !== r.name && <span style={{ ...muted, display: "block" }}>{r.code}</span>}
            </td>
            <td>{sourceLabel(r.source)}</td>
            {kind !== "index" && <td>{r.currency ?? "—"}</td>}
            <td className="num" style={right}>{r.latest ?? "—"}</td>
            <td className="num">{period(r.latest_date)}</td>
            <td className="num">{period(r.first_date)}</td>
            <td className="num" style={right}>{r.count.toLocaleString("en-US")}</td>
            {kind !== "index" && (
              <td className="num" style={right} data-testid="held-gaps">
                {r.held_gaps === null ? <span style={muted}>–</span> : r.held_gaps > 0 ? <Badge tone="warn">{r.held_gaps}</Badge> : "0"}
              </td>
            )}
            <td>
              <span style={{ display: "inline-flex", gap: 6, flexWrap: "wrap" }}>
                {r.stale && <Badge tone="warn">Stale</Badge>}
                {r.edits > 0 && <Badge tone="info">{r.edits} edited</Badge>}
                {r.used && <span style={muted}>in use</span>}
              </span>
            </td>
          </tr>
        ))}
      </tbody>
    </DataTable>
  );
}

// ------------------------------------------------------------------ detail

function ValueRow({ p, kind, seriesKey, manualOnly }: { p: SeriesPoint; kind: SeriesKindName; seriesKey: string; manualOnly: boolean }) {
  const m = useMarketMutations(kind, seriesKey);
  const [editing, setEditing] = useState(false);
  const [value, setValue] = useState(p.value);
  const error = m.set.error ?? m.ignore.error ?? m.revert.error;
  return (
    <tr data-testid="value-row" style={p.in_use ? undefined : { color: "var(--text-muted)", textDecoration: "line-through" }}>
      <td className="num">{period(p.period)}</td>
      <td className="num" style={right}>
        {editing ? (
          <input
            aria-label={`New value for ${p.period}`}
            className={`${s.control} ${s.number}`}
            style={{ width: 120, textAlign: "right" }}
            value={value}
            onChange={(e) => setValue(e.target.value)}
          />
        ) : (
          <>
            {p.value}
            {p.original && <span style={{ ...muted, display: "block" }}>was {p.original}</span>}
          </>
        )}
      </td>
      <td>{p.source}</td>
      <td>
        {p.tag === "edited" && <Badge tone="info">Edited</Badge>}
        {p.tag === "ignored" && <Badge tone="warn">Ignored</Badge>}
      </td>
      <td style={muted}>{p.fetched_at ? p.fetched_at.slice(0, 16).replace("T", " ") : ""}</td>
      <td style={{ ...right, whiteSpace: "nowrap" }}>
        {editing ? (
          <>
            <Button
              small
              variant="primary"
              disabled={m.set.isPending}
              onClick={() =>
                m.set.mutate({ period: p.period, value, currency: kind === "price" ? p.currency : null }, { onSuccess: () => setEditing(false) })
              }
            >
              Save
            </Button>{" "}
            <Button small variant="ghost" onClick={() => setEditing(false)}>
              Cancel
            </Button>
          </>
        ) : (
          <>
            <Button small variant="ghost" onClick={() => setEditing(true)}>
              Edit
            </Button>
            {!manualOnly && p.tag !== "ignored" && (
              <Button small variant="ghost" onClick={() => m.ignore.mutate(p.period)}>
                Ignore
              </Button>
            )}
            {(p.tag || (manualOnly && p.source === "manual")) && (
              <Button small variant="ghost" onClick={() => m.revert.mutate(p.period)}>
                {manualOnly ? "Delete" : "Revert"}
              </Button>
            )}
          </>
        )}
        {error && <span className={s.error} style={{ display: "block" }}>{errorMessage(error)}</span>}
      </td>
    </tr>
  );
}

function AddValue({ kind, seriesKey, currency }: { kind: SeriesKindName; seriesKey: string; currency: string | null }) {
  const m = useMarketMutations(kind, seriesKey);
  const [when, setWhen] = useState("");
  const [value, setValue] = useState("");
  const [ccy, setCcy] = useState(currency ?? "");
  return (
    <form
      aria-label="Add value"
      className={s.toolbar}
      onSubmit={(e) => {
        e.preventDefault();
        m.set.mutate({ period: when, value, currency: kind === "price" ? ccy : null }, { onSuccess: () => setValue("") });
      }}
    >
      <input
        aria-label={kind === "index" ? "Month" : "Date"}
        type={kind === "index" ? "month" : "date"}
        className={`${s.control} ${s.date}`}
        value={when}
        onChange={(e) => setWhen(e.target.value)}
      />
      <input aria-label="Value" inputMode="decimal" placeholder="Value" className={`${s.control} ${s.number}`} style={{ width: 140 }} value={value} onChange={(e) => setValue(e.target.value)} />
      {kind === "price" && <input aria-label="Currency" maxLength={3} className={s.control} style={{ width: 70 }} value={ccy} onChange={(e) => setCcy(e.target.value.toUpperCase())} />}
      <Button type="submit" disabled={!when || !value || m.set.isPending}>
        Add value
      </Button>
      {m.set.error && <span className={s.error}>{errorMessage(m.set.error)}</span>}
    </form>
  );
}

function Detail({ kind, d, more, onClose }: { kind: SeriesKindName; d: SeriesDetail; more: () => void; onClose: () => void }) {
  const m = useMarketMutations(kind, d.row.key);
  const exportCsv = async () => {
    const dir = await open({ directory: true, title: "Choose a folder for the CSV file" });
    if (typeof dir !== "string") return;
    m.exportCsv.mutate(dir);
  };
  const chart = d.chart.map((p) => Number(p.value));
  return (
    <Card
      title={d.row.name}
      label="Series detail"
      aside={
        <span style={{ display: "inline-flex", gap: 8, alignItems: "center" }}>
          <Button small onClick={() => void exportCsv()} disabled={m.exportCsv.isPending}>
            Export CSV
          </Button>
          <Button small variant="ghost" onClick={onClose}>
            Close
          </Button>
        </span>
      }
    >
      <p style={{ ...muted, margin: 0 }}>
        {sourceLabel(d.row.source)} · {d.row.key}
        {d.row.currency ? ` · ${d.row.currency}` : ""} · {d.total.toLocaleString("en-US")} values.{" "}
        {d.manual_only
          ? "This series exists only in this database; its values are manual prices on the asset."
          : "Edits stay in this database; the shared cache keeps the provider's values."}
      </p>
      {kind !== "index" && (
        <p style={{ fontSize: 13, margin: 0 }} data-testid="gap-counts">
          Gaps while held:{" "}
          <strong>{d.row.held_gaps === null ? "not held" : d.row.held_gaps.toLocaleString("en-US")}</strong>
          <span style={muted}> (weekdays you held it without a value, including before the history starts)</span>
          {" · "}Series gaps: <strong>{d.row.series_gaps.toLocaleString("en-US")}</strong>
          <span style={muted}> (weekdays between its first and last value without one)</span>
        </p>
      )}
      {m.exportCsv.data && (
        <p style={{ ...muted, margin: 0 }} data-testid="export-done">
          Saved {m.exportCsv.data.split(/[\\/]/).pop()}
        </p>
      )}
      {chart.length > 1 ? (
        <LineChart
          label={`${d.row.name} history`}
          dates={d.chart.map((p) => p.period)}
          series={[{ name: d.row.name, values: chart, labels: d.chart.map((p) => p.value) }]}
          format={(v) => (Math.abs(v) >= 100 ? v.toFixed(0) : v.toPrecision(4))}
          formatDate={period}
        />
      ) : (
        <EmptyState title="Not enough values to chart" message="Add values below, or refresh to download history." />
      )}
      <AddValue kind={kind} seriesKey={d.row.key} currency={d.row.currency} />
      <DataTable label="Values" minWidth={720}>
        <thead>
          <tr>
            <th scope="col">{kind === "index" ? "Month" : "Date"}</th>
            <th scope="col" style={right}>Value</th>
            <th scope="col">Source</th>
            <th scope="col">Tag</th>
            <th scope="col">Fetched</th>
            <th scope="col" style={right}>
              <span className="visually-hidden">Actions</span>
            </th>
          </tr>
        </thead>
        <tbody>
          {d.points.map((p) => (
            <ValueRow key={p.period} p={p} kind={kind} seriesKey={d.row.key} manualOnly={d.manual_only} />
          ))}
        </tbody>
      </DataTable>
      {d.points.length < d.total && (
        <Button onClick={more}>Show more ({d.total - d.points.length} left)</Button>
      )}
    </Card>
  );
}

// ------------------------------------------------------------------ screen

export function MarketData() {
  // /market-data?tab=providers opens the Providers tab (Settings links there).
  const [params] = useSearchParams();
  const [tab, setTab] = useState<Tab>(params.get("tab") === "providers" ? "providers" : "price");
  if (tab === "providers") {
    return (
      <>
        <PageHeader
          title="Market data"
          subtitle="Where prices, exchange rates and inflation indices come from"
          actions={<Segmented label="Market data" value={tab} onChange={setTab} options={TABS} />}
        />
        <ProvidersTab />
      </>
    );
  }
  return <SeriesBrowser key={tab} kind={tab} onTab={setTab} />;
}

function SeriesBrowser({ kind, onTab }: { kind: SeriesKindName; onTab: (t: Tab) => void }) {
  // Links from Holdings open one price series: /market-data?series=<key>.
  const [params] = useSearchParams();
  const [filter, setFilter] = useState<SeriesFilter>(EMPTY_FILTER);
  const [open_, setOpen] = useState<string | null>(kind === "price" ? params.get("series") : null);
  const [pages, setPages] = useState(1);
  const { data } = useMarketSeries(kind, filter);
  const { data: detail } = useMarketDetail(kind, open_, pages);
  const set = (f: Partial<SeriesFilter>) => setFilter({ ...filter, ...f });
  const filtered = JSON.stringify(filter) !== JSON.stringify(EMPTY_FILTER);
  return (
    <>
      <PageHeader
        title="Market data"
        subtitle={data ? `${data.rows.length} of ${data.total} series · edits stay in this database` : undefined}
        actions={
          <Segmented
            label="Market data"
            value={kind}
            onChange={onTab}
            options={TABS}
          />
        }
      />
      <section aria-label="Filters" className={s.toolbar}>
        <select aria-label="Source" className={s.control} value={filter.source ?? ""} onChange={(e) => set({ source: e.target.value || null })}>
          <option value="">All sources</option>
          {data?.sources.map((x) => (
            <option key={x} value={x}>
              {sourceLabel(x)}
            </option>
          ))}
        </select>
        {kind !== "index" && (
          <select aria-label="Currency" className={s.control} value={filter.currency ?? ""} onChange={(e) => set({ currency: e.target.value || null })}>
            <option value="">All currencies</option>
            {data?.currencies.map((x) => (
              <option key={x} value={x}>
                {x}
              </option>
            ))}
          </select>
        )}
        <input
          aria-label="Search series"
          type="search"
          placeholder="Name, code or symbol"
          className={`${s.control} ${s.search}`}
          value={filter.text ?? ""}
          onChange={(e) => set({ text: e.target.value || null })}
        />
        <Toggle label="Used by my assets" on={filter.used} set={(v) => set({ used: v })} />
        <Toggle label="Has edits" on={filter.edited} set={(v) => set({ edited: v })} />
        {kind !== "index" && <Toggle label="Stale or gaps" on={filter.attention} set={(v) => set({ attention: v })} />}
        {filtered && (
          <Button small variant="ghost" onClick={() => setFilter(EMPTY_FILTER)}>
            Clear filters
          </Button>
        )}
        {data && (
          <span style={muted} data-testid="series-count">
            {data.rows.length} series match
          </span>
        )}
      </section>
      {detail && open_ && (
        <Detail
          kind={kind}
          d={detail}
          more={() => setPages(pages + 1)}
          onClose={() => {
            setOpen(null);
            setPages(1);
          }}
        />
      )}
      {data && data.total === 0 ? (
        <EmptyState
          title="No market data yet"
          message="Refresh prices, or assign price sources on Assets; the history downloads in the background."
          action={{ to: "/assets", label: "Go to Assets" }}
        />
      ) : data && data.rows.length === 0 ? (
        <Card>
          <p style={{ margin: 0, fontSize: 13 }}>No series match these filters.</p>
        </Card>
      ) : (
        data && (
          <Card label="Series">
            <SeriesTable
              rows={data.rows}
              kind={kind}
              onOpen={(key) => {
                setOpen(key);
                setPages(1);
              }}
            />
            {kind !== "index" && (
              <p style={{ ...muted, margin: 0 }}>
                Gaps while held count weekdays you held the instrument or currency without a value, including days before its history starts.
                Public holidays count, since no prices or rates are published then. “–” means nothing in this database holds it.
              </p>
            )}
          </Card>
        )
      )}
    </>
  );
}
