import { useNavigate } from "react-router";
import type { AddActivityRequest } from "../app/ViewBar";
import type { Position } from "../bridge/generated/Position";
import { useMarketDetail } from "../bridge/queries";
import { Badge, Button, Card, LineChart } from "../ui";
import { CLASS_LABELS, longDate } from "./format";

/** A day a year before `iso`. */
function yearBefore(iso: string): string {
  const [y, m, d] = iso.split("-");
  return `${Number(y) - 1}-${m}-${d}`;
}

function PriceChart({ p }: { p: Position }) {
  const key = p.series_key;
  const primary = useMarketDetail("price", key, 1);
  // A series kept only in this database is listed as asset:<id> on Market data.
  const fallbackKey = primary.error && p.asset_id !== null && key !== `asset:${p.asset_id}` ? `asset:${p.asset_id}` : null;
  const fallback = useMarketDetail("price", fallbackKey, 1);
  const detail = fallbackKey ? fallback.data : primary.data;
  const usedKey = fallbackKey ?? key;
  if (!key || !p.price) {
    return <p style={{ margin: 0, fontSize: 13, color: "var(--text-muted)" }}>No prices are stored for this asset. Choose a price source or add a manual price on Assets.</p>;
  }
  if (!detail) return null;
  const last = detail.chart[detail.chart.length - 1]?.period;
  const from = last ? yearBefore(last) : "";
  const points = detail.chart.filter((c) => c.period >= from);
  if (points.length === 0) return <p style={{ margin: 0, fontSize: 13, color: "var(--text-muted)" }}>No prices in the last year.</p>;
  return (
    <>
      <LineChart
        label={`${p.symbol ?? p.name ?? "Price"} price`}
        dates={points.map((c) => c.period)}
        series={[{ name: "Price", values: points.map((c) => Number(c.value)), labels: points.map((c) => `${c.value} ${detail.row.currency ?? ""}`.trim()) }]}
        format={(v) => v.toLocaleString("en-US", { maximumFractionDigits: 4 })}
        formatDate={longDate}
      />
      <span hidden data-testid="series-key">
        {usedKey}
      </span>
    </>
  );
}

/** The selected position's figures, price source, lots and price chart, with links. */
export function PositionInspector({ p, accountId, accountName, onClose }: { p: Position; accountId: number; accountName: string; onClose: () => void }) {
  const navigate = useNavigate();
  const title = p.symbol ?? p.name ?? p.instrument_key;
  const facts: [string, string][] = [
    ["Value", p.value && p.price ? `${p.value} ${p.price.currency}` : p.value_note ?? "—"],
    ["Unrealized", p.gain ? `${p.gain.amount} ${p.gain.currency} (${p.gain.pct})` : p.gain_note ?? "—"],
    ["Quantity", p.quantity],
    ["Average cost", p.cost.average ? `${p.cost.average} ${p.cost.currency ?? ""}`.trim() : "not available"],
  ];
  const text = p.symbol ?? p.isin ?? p.name ?? "";
  return (
    <Card
      label="Inspector"
      title={title}
      aside={
        <Button small variant="ghost" aria-label="Close inspector" onClick={onClose}>
          ✕
        </Button>
      }
    >
      <div style={{ display: "flex", flexDirection: "column", gap: 14 }}>
        <div style={{ fontSize: 12.5, color: "var(--text-muted)" }}>
          {p.name && p.name !== title ? `${p.name} · ` : ""}
          {accountName} · {CLASS_LABELS[p.class] ?? p.class}
        </div>
        <PriceChart key={p.instrument_key} p={p} />
        <dl style={{ display: "grid", gridTemplateColumns: "repeat(2, minmax(0, 1fr))", gap: 10, margin: 0 }}>
          {facts.map(([k, v]) => (
            <div key={k} style={{ background: "var(--surface-sunken)", borderRadius: 8, padding: "10px 12px" }}>
              <dt style={{ fontSize: 12, color: "var(--text-muted)" }}>{k}</dt>
              <dd className="num" style={{ margin: 0, fontSize: 14 }}>
                {v}
              </dd>
            </div>
          ))}
        </dl>
        <dl style={{ margin: 0, fontSize: 13 }}>
          <Row k="Price source" v={p.price?.source ?? "none"} />
          <Row
            k="Latest price"
            v={
              p.price ? (
                <>
                  {p.price.stale && <Badge tone="warn">stale</Badge>} {p.price.value} {p.price.currency} on {longDate(p.price.date)}
                </>
              ) : (
                "no price"
              )
            }
          />
          <Row k="Open lots" v={p.open_lots === null ? "not available for short positions" : String(p.open_lots)} />
        </dl>
        <div style={{ display: "flex", flexWrap: "wrap", gap: 8 }}>
          <Button small onClick={() => navigate(`/activity?account=${accountId}&text=${encodeURIComponent(text)}`)}>
            Activity
          </Button>
          {p.series_key && (
            <Button small onClick={() => navigate(`/market-data?series=${encodeURIComponent(p.series_key!)}`)}>
              Market data
            </Button>
          )}
          <Button
            small
            onClick={() => navigate("/activity", { state: { add: { accountId, at: Date.now(), assetId: p.asset_id } } satisfies AddActivityRequest })}
          >
            Add activity
          </Button>
        </div>
      </div>
    </Card>
  );
}

function Row({ k, v }: { k: string; v: React.ReactNode }) {
  return (
    <div style={{ display: "flex", justifyContent: "space-between", gap: 12, padding: "8px 0", borderBottom: "1px solid var(--divider)" }}>
      <dt style={{ color: "var(--text-secondary)" }}>{k}</dt>
      <dd style={{ margin: 0, textAlign: "right" }}>{v}</dd>
    </div>
  );
}
