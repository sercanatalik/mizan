import { useState } from "react";
import { useLocation } from "react-router";
import { PageHeader } from "../app/Layout";
import type { EditAssetRequest, ManualPriceRequest } from "../app/ViewBar";
import { errorMessage } from "../bridge/api";
import type { PriceSourceKind } from "../bridge/api";
import type { AssetView } from "../bridge/generated/AssetView";
import type { ProviderView } from "../bridge/generated/ProviderView";
import { useAssetMutations, useAssets, useSettings } from "../bridge/queries";
import { Badge, Button, Card, DataTable, EmptyState } from "../ui";
import { longDate } from "./format";
import { RefreshButton } from "./RefreshButton";

const input = { font: "inherit", fontSize: 13, padding: "5px 8px", borderRadius: 6, border: "1px solid var(--border-strong)" } as const;

const SOURCE_OPTIONS: { kind: PriceSourceKind; label: string }[] = [
  { kind: "none", label: "None" },
  { kind: "manual", label: "Manual" },
  { kind: "tefas", label: "TEFAS" },
  { kind: "yahoo", label: "Yahoo Finance" },
  { kind: "twelvedata", label: "Twelve Data" },
];

const BADGE_NAME: Record<string, string> = { tefas: "TEFAS", yahoo: "Yahoo", twelvedata: "Twelve Data" };

const PLACEHOLDER: Partial<Record<PriceSourceKind, string>> = { tefas: "Code", yahoo: "e.g. VWCE.DE", twelvedata: "e.g. MSFT" };

const hasSymbol = (k: PriceSourceKind) => k === "tefas" || k === "yahoo" || k === "twelvedata";

/** The provider menu: choose a price source and, for providers, the fund code or symbol. */
function SourceEditor({ a }: { a: AssetView }) {
  const m = useAssetMutations();
  const [kind, setKind] = useState<PriceSourceKind>(a.source.kind);
  const [symbol, setSymbol] = useState(a.source.symbol ?? "");
  const [missing, setMissing] = useState(false);
  const busy = m.setPriceSource.isPending;
  const save = () => {
    const needs = hasSymbol(kind);
    if (needs && !symbol.trim()) {
      setMissing(true);
      return;
    }
    setMissing(false);
    m.setPriceSource.mutate({ assetId: a.asset_id, kind, symbol: needs ? symbol.trim() : null });
  };
  return (
    <span style={{ display: "inline-flex", flexWrap: "wrap", gap: 6, alignItems: "center" }}>
      <label className="visually-hidden" htmlFor={`kind-${a.asset_id}`}>
        Price source for {a.label}
      </label>
      <select
        id={`kind-${a.asset_id}`}
        style={input}
        value={kind}
        onChange={(e) => {
          setKind(e.target.value as PriceSourceKind);
          setMissing(false);
        }}
      >
        {SOURCE_OPTIONS.map((o) => (
          <option key={o.kind} value={o.kind}>
            {o.label}
          </option>
        ))}
      </select>
      {hasSymbol(kind) && (
        <>
          <label className="visually-hidden" htmlFor={`symbol-${a.asset_id}`}>
            {kind === "tefas" ? `TEFAS fund code for ${a.label}` : `Symbol for ${a.label}`}
          </label>
          <input
            id={`symbol-${a.asset_id}`}
            style={{ ...input, width: kind === "tefas" ? 64 : 120 }}
            value={symbol}
            placeholder={PLACEHOLDER[kind]}
            onChange={(e) => setSymbol(e.target.value.toUpperCase())}
          />
        </>
      )}
      <Button small disabled={busy} onClick={save}>
        Save source
      </Button>
      {missing && (
        <span role="alert" style={{ fontSize: 12, color: "var(--warn-strong)" }}>
          A symbol is required
        </span>
      )}
      {m.setPriceSource.error && (
        <span role="alert" style={{ fontSize: 12, color: "var(--warn-strong)" }}>
          {errorMessage(m.setPriceSource.error)}
        </span>
      )}
    </span>
  );
}

function ManualPriceForm({ a }: { a: AssetView }) {
  const m = useAssetMutations();
  const today = new Date().toISOString().slice(0, 10);
  const [date, setDate] = useState(today);
  const [value, setValue] = useState("");
  const [currency, setCurrency] = useState(a.currency ?? "TRY");
  return (
    <form
      style={{ display: "inline-flex", flexWrap: "wrap", gap: 6, alignItems: "center" }}
      onSubmit={(e) => {
        e.preventDefault();
        m.addManualPrice.mutate({ assetId: a.asset_id, date, value: value.trim(), currency: currency.trim() }, { onSuccess: () => setValue("") });
      }}
    >
      <label className="visually-hidden" htmlFor={`mp-date-${a.asset_id}`}>
        Price date for {a.label}
      </label>
      <input id={`mp-date-${a.asset_id}`} type="date" style={input} value={date} onChange={(e) => setDate(e.target.value)} />
      <label className="visually-hidden" htmlFor={`mp-value-${a.asset_id}`}>
        Price for {a.label}
      </label>
      <input id={`mp-value-${a.asset_id}`} style={{ ...input, width: 90 }} inputMode="decimal" placeholder="Price" value={value} onChange={(e) => setValue(e.target.value)} />
      <label className="visually-hidden" htmlFor={`mp-ccy-${a.asset_id}`}>
        Currency for {a.label}
      </label>
      <input id={`mp-ccy-${a.asset_id}`} style={{ ...input, width: 56 }} value={currency} onChange={(e) => setCurrency(e.target.value.toUpperCase())} />
      <Button small type="submit" disabled={!value.trim() || m.addManualPrice.isPending}>
        Add price
      </Button>
      {m.addManualPrice.error && <span role="alert" style={{ fontSize: 12, color: "var(--warn-strong)" }}>{errorMessage(m.addManualPrice.error)}</span>}
    </form>
  );
}

function MergeControl({ a, all }: { a: AssetView; all: AssetView[] }) {
  const m = useAssetMutations();
  const [into, setInto] = useState("");
  if (a.merged_into) {
    return (
      <span style={{ display: "inline-flex", gap: 6, alignItems: "center" }}>
        <Badge>Merged into {a.merged_into.label}</Badge>
        <Button small variant="ghost" onClick={() => m.undoMerge.mutate(a.merged_into!.merge_id)}>
          Undo merge
        </Button>
      </span>
    );
  }
  const targets = all.filter((x) => x.asset_id !== a.asset_id && !x.merged_into);
  if (targets.length === 0) return null;
  return (
    <span style={{ display: "inline-flex", gap: 6, alignItems: "center" }}>
      <label className="visually-hidden" htmlFor={`merge-${a.asset_id}`}>
        Merge {a.label} into
      </label>
      <select id={`merge-${a.asset_id}`} style={input} value={into} onChange={(e) => setInto(e.target.value)}>
        <option value="">Merge into…</option>
        {targets.map((t) => (
          <option key={t.asset_id} value={t.asset_id}>
            {t.label} ({t.instrument_key})
          </option>
        ))}
      </select>
      <Button small disabled={!into || m.mergeAssets.isPending} onClick={() => m.mergeAssets.mutate({ from: a.asset_id, into: Number(into) })}>
        Merge
      </Button>
    </span>
  );
}

/** Why a provider source will not refresh right now, if it will not. */
function providerNote(a: AssetView, providers: ProviderView[]): string | null {
  const p = providers.find((x) => x.id === a.source.kind);
  if (!p) return null;
  if (!p.enabled) return "provider off in Settings";
  if (p.needs_api_key && !p.api_key_hint) return "needs an API key";
  return null;
}

function SourceCell({ a, providers }: { a: AssetView; providers: ProviderView[] }) {
  const m = useAssetMutations();
  const name = BADGE_NAME[a.source.kind];
  const note = providerNote(a, providers);
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
      <span style={{ display: "inline-flex", flexWrap: "wrap", gap: 6, alignItems: "center" }}>
        {name ? (
          <Badge tone="accent">
            {name} · {a.source.symbol}
          </Badge>
        ) : a.source.kind === "manual" ? (
          <Badge>Manual</Badge>
        ) : (
          <Badge>None</Badge>
        )}
        {note && (
          <span style={{ fontSize: 12, color: "var(--text-muted)" }} data-testid="source-note">
            {note}
          </span>
        )}
      </span>
      {a.suggestion && (
        <span style={{ display: "inline-flex", gap: 6, alignItems: "center", fontSize: 12 }}>
          Suggested: {a.suggestion.label}
          <Button small variant="primary" onClick={() => m.acceptSuggestion.mutate(a.asset_id)}>
            Accept
          </Button>
        </span>
      )}
    </div>
  );
}

const BENCHMARK_KINDS = [
  { value: "yahoo", label: "Yahoo Finance symbol", placeholder: "SPY, VWCE.DE, XU100.IS" },
  { value: "twelvedata", label: "Twelve Data symbol", placeholder: "SPY" },
  { value: "tefas", label: "TEFAS fund code", placeholder: "AFT" },
] as const;

/** Add an instrument to compare against, whether or not you hold it. */
function AddBenchmark() {
  const m = useAssetMutations();
  const [kind, setKind] = useState<(typeof BENCHMARK_KINDS)[number]["value"]>("yahoo");
  const [symbol, setSymbol] = useState("");
  const [name, setName] = useState("");
  const current = BENCHMARK_KINDS.find((k) => k.value === kind)!;
  return (
    <Card
      title="Benchmarks"
      label="Benchmarks"
      aside={<span style={{ fontSize: 12, color: "var(--text-muted)" }}>Marked assets are offered on Performance, with price history from your first activity</span>}
    >
      <form
        style={{ display: "flex", flexWrap: "wrap", gap: 8, alignItems: "center" }}
        onSubmit={(e) => {
          e.preventDefault();
          m.addBenchmarkInstrument.mutate(
            { kind, symbol: symbol.trim(), name: name.trim() || null },
            {
              onSuccess: () => {
                setSymbol("");
                setName("");
              },
            },
          );
        }}
      >
        <label style={{ display: "inline-flex", gap: 6, alignItems: "center", fontSize: 13 }}>
          Source
          <select style={input} value={kind} onChange={(e) => setKind(e.target.value as typeof kind)}>
            {BENCHMARK_KINDS.map((k) => (
              <option key={k.value} value={k.value}>
                {k.label}
              </option>
            ))}
          </select>
        </label>
        <label style={{ display: "inline-flex", gap: 6, alignItems: "center", fontSize: 13 }}>
          Symbol
          <input style={{ ...input, width: 140 }} placeholder={current.placeholder} value={symbol} onChange={(e) => setSymbol(e.target.value)} />
        </label>
        <label style={{ display: "inline-flex", gap: 6, alignItems: "center", fontSize: 13 }}>
          Name
          <input style={{ ...input, width: 200 }} placeholder="Optional" value={name} onChange={(e) => setName(e.target.value)} />
        </label>
        <Button small type="submit" variant="primary" disabled={!symbol.trim() || m.addBenchmarkInstrument.isPending}>
          Add benchmark
        </Button>
        {m.addBenchmarkInstrument.isSuccess && <span style={{ fontSize: 12, color: "var(--gain)" }}>Added; its price history is downloading</span>}
        {m.addBenchmarkInstrument.error && (
          <span role="alert" style={{ fontSize: 12, color: "var(--warn-strong)" }}>
            {errorMessage(m.addBenchmarkInstrument.error)}
          </span>
        )}
      </form>
    </Card>
  );
}

/** Add manual price, opened from the Add menu: pick an asset, then its price form. */
function ManualPricePicker({ assets, onClose }: { assets: AssetView[]; onClose: () => void }) {
  const [assetId, setAssetId] = useState<number | null>(null);
  const chosen = assets.find((a) => a.asset_id === assetId) ?? null;
  return (
    <Card title="Add manual price" label="Add manual price" aside={<Button small variant="ghost" onClick={onClose}>Close</Button>}>
      <div style={{ display: "flex", flexWrap: "wrap", gap: 10, alignItems: "center" }}>
        <label style={{ display: "inline-flex", gap: 8, alignItems: "center", fontSize: 13 }}>
          Asset
          <select style={input} value={assetId ?? ""} onChange={(e) => setAssetId(e.target.value ? Number(e.target.value) : null)}>
            <option value="">Choose an asset…</option>
            {assets.map((a) => (
              <option key={a.asset_id} value={a.asset_id}>
                {a.label}
              </option>
            ))}
          </select>
        </label>
        {chosen && <ManualPriceForm key={chosen.asset_id} a={chosen} />}
      </div>
    </Card>
  );
}

export function Assets() {
  const request = (useLocation().state as ManualPriceRequest | null)?.manualPrice ?? null;
  const [closedFor, setClosedFor] = useState<number | null>(null);
  const picker = request !== null && request !== closedFor;
  const { data } = useAssets();
  const { data: settings } = useSettings();
  const providers = settings?.providers ?? [];
  const m = useAssetMutations();
  const editRequest = (useLocation().state as EditAssetRequest | null)?.editAsset ?? null;
  const [editing, setEditing] = useState<number | null>(editRequest);
  if (!data) return <PageHeader title="Assets" />;
  return (
    <>
      <PageHeader title="Assets" subtitle="Price sources, manual prices and duplicate merging" actions={<RefreshButton />} />
      {picker && data.length > 0 && <ManualPricePicker assets={data} onClose={() => setClosedFor(request)} />}
      <AddBenchmark />
      {data.length === 0 ? (
        <EmptyState title="No assets yet" message="Assets are created when you import statements." action={{ to: "/imports", label: "Go to Imports" }} />
      ) : (
        <Card label="Assets">
          <DataTable label="Assets" minWidth={960}>
            <thead>
              <tr>
                <th scope="col">Asset</th>
                <th scope="col">Class</th>
                <th scope="col">Price source</th>
                <th scope="col" style={{ textAlign: "right" }}>Latest price</th>
                <th scope="col" style={{ textAlign: "right" }}>Accounts</th>
                <th scope="col" style={{ textAlign: "center" }}>Benchmark</th>
                <th scope="col" style={{ textAlign: "right" }}>
                  <span className="visually-hidden">Actions</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {data.map((a) => (
                <tr key={a.asset_id} data-testid="asset-row">
                  <td>
                    <div style={{ fontWeight: 500 }}>{a.label}</div>
                    {a.name && a.name !== a.label && <div style={{ fontSize: 12, color: "var(--text-muted)" }}>{a.name}</div>}
                    <div className="num" style={{ fontSize: 12, color: "var(--text-muted)" }}>
                      {a.instrument_key}
                    </div>
                    {editing === a.asset_id && (
                      <div style={{ display: "flex", flexDirection: "column", gap: 8, marginTop: 8 }}>
                        <SourceEditor a={a} />
                        <ManualPriceForm a={a} />
                        {a.manual_prices.map((p) => (
                          <span key={p.date} style={{ display: "inline-flex", gap: 6, alignItems: "center", fontSize: 12 }}>
                            Manual {p.value} {p.currency} on {p.date}
                            <Button small variant="ghost" onClick={() => m.deleteManualPrice.mutate({ assetId: a.asset_id, date: p.date })}>
                              Delete
                            </Button>
                          </span>
                        ))}
                        <MergeControl a={a} all={data} />
                      </div>
                    )}
                  </td>
                  <td style={{ color: "var(--text-secondary)" }}>{a.class}</td>
                  <td>
                    <SourceCell a={a} providers={providers} />
                    {a.merged_into && editing !== a.asset_id && <Badge>Merged into {a.merged_into.label}</Badge>}
                  </td>
                  <td className="num" style={{ textAlign: "right" }}>
                    {a.latest_price ? (
                      <>
                        <div>
                          {a.latest_price.value} {a.latest_price.currency}
                        </div>
                        <div style={{ fontSize: 12, color: a.latest_price.stale ? "var(--warn-strong)" : "var(--text-muted)" }}>
                          {a.latest_price.stale ? "stale · " : ""}
                          {longDate(a.latest_price.date)}
                        </div>
                      </>
                    ) : (
                      <span style={{ color: "var(--text-muted)" }}>no price</span>
                    )}
                  </td>
                  <td className="num" style={{ textAlign: "right" }}>
                    {a.holders}
                  </td>
                  <td style={{ textAlign: "center" }}>
                    {!a.merged_into && (
                      <input
                        type="checkbox"
                        role="switch"
                        checked={a.benchmark}
                        aria-label={`Use ${a.label} as a benchmark`}
                        disabled={m.setBenchmarkCandidate.isPending}
                        onChange={(e) => m.setBenchmarkCandidate.mutate({ assetId: a.asset_id, on: e.target.checked })}
                      />
                    )}
                  </td>
                  <td style={{ textAlign: "right" }}>
                    <Button small variant="ghost" aria-expanded={editing === a.asset_id} onClick={() => setEditing(editing === a.asset_id ? null : a.asset_id)}>
                      {editing === a.asset_id ? "Done" : "Edit"}
                    </Button>
                  </td>
                </tr>
              ))}
            </tbody>
          </DataTable>
        </Card>
      )}
    </>
  );
}
