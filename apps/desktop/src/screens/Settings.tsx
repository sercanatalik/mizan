import { useState } from "react";
import { PageHeader } from "../app/Layout";
import { errorMessage } from "../bridge/api";
import type { ProviderView } from "../bridge/generated/ProviderView";
import type { RefreshReport } from "../bridge/generated/RefreshReport";
import type { SourceProgress } from "../bridge/generated/SourceProgress";
import { useBackfillStatus, useSettings, useSettingsMutations } from "../bridge/queries";
import { Badge, Button, Card, DataTable } from "../ui";
import { RefreshButton } from "./RefreshButton";

const STATUS: Record<string, { label: string; tone: "accent" | "warn" | "neutral" }> = {
  ok: { label: "OK", tone: "accent" },
  failed: { label: "Failed", tone: "warn" },
  disabled: { label: "Disabled", tone: "neutral" },
  idle: { label: "Nothing to fetch", tone: "neutral" },
  needs_key: { label: "Needs an API key", tone: "warn" },
};

function when(ts: string | null | undefined): string {
  return ts ? ts.replace("T", " ").replace(/:\d\dZ$/, " UTC").replace("Z", " UTC") : "never";
}

function Report({ report }: { report: RefreshReport }) {
  return (
    <ul style={{ margin: 0, padding: 0, listStyle: "none" }} aria-label="Last refresh">
      {report.sources.map((s) => (
        <li key={s.id} style={{ padding: "8px 0", borderTop: "1px solid var(--divider)", display: "flex", gap: 10, alignItems: "baseline", flexWrap: "wrap" }}>
          <strong style={{ minWidth: 60 }}>{s.name}</strong>
          <Badge tone={STATUS[s.status]?.tone ?? "neutral"}>{STATUS[s.status]?.label ?? s.status}</Badge>
          {s.updated > 0 && <span style={{ fontSize: 12, color: "var(--text-muted)" }}>{s.updated} updated</span>}
          {s.messages.map((m) => (
            <span key={m} style={{ fontSize: 12, color: s.status === "failed" ? "var(--warn-strong)" : "var(--text-muted)" }}>
              {m}
            </span>
          ))}
        </li>
      ))}
    </ul>
  );
}

const HISTORY: Record<string, { label: string; tone: "accent" | "warn" | "neutral" }> = {
  running: { label: "Downloading", tone: "accent" },
  done: { label: "Complete", tone: "accent" },
  waiting: { label: "Waiting", tone: "neutral" },
  failed: { label: "Failed", tone: "warn" },
  disabled: { label: "Disabled", tone: "neutral" },
  idle: { label: "Not needed", tone: "neutral" },
  needs_key: { label: "Waiting for an API key", tone: "neutral" },
};

/** One line describing a provider's history backfill. */
export function historyText(s: SourceProgress): string {
  const unit = s.unit === "days" || s.unit === "years" ? s.unit : "months";
  switch (s.state) {
    case "running":
      return `${s.done} of ${s.planned} ${unit}`;
    case "done":
      return "All history needed is downloaded";
    case "waiting":
      return s.planned > 0 ? `${s.planned} ${unit} missing · continues after the next refresh` : "Continues after the next refresh";
    case "failed":
      return s.error ?? "Failed";
    case "disabled":
      return "Provider is off; no history is downloaded";
    case "needs_key":
      return `Save a ${s.name} API key above to download history`;
    case "idle":
      if (s.id === "tcmb") return "Only TRY is used, or nothing is imported yet; no exchange rates needed";
      if (s.id === "tefas") return "Nothing to download: no fund priced from TEFAS has activities yet";
      return `Nothing to download: no asset priced from ${s.name} has activities yet`;
  }
}

/** Save or clear a provider's API key. Only a masked hint of a saved key is ever shown. */
function ApiKeyField({ p }: { p: ProviderView }) {
  const m = useSettingsMutations();
  const [key, setKey] = useState("");
  const id = `api-key-${p.id}`;
  return (
    <div style={{ display: "flex", flexWrap: "wrap", gap: 6, alignItems: "center", marginTop: 6 }}>
      <label htmlFor={id} style={{ fontSize: 12 }}>
        {p.name} API key
      </label>
      {p.api_key_hint ? (
        <>
          <code data-testid="api-key-hint">{p.api_key_hint}</code>
          <Button small variant="ghost" disabled={m.setApiKey.isPending} onClick={() => m.setApiKey.mutate({ id: p.id, key: null })}>
            Clear key
          </Button>
        </>
      ) : (
        <span style={{ fontSize: 12, color: "var(--text-muted)" }}>not saved</span>
      )}
      <input
        id={id}
        type="password"
        autoComplete="off"
        spellCheck={false}
        value={key}
        placeholder={p.api_key_hint ? "Replace key" : "Paste your key"}
        onChange={(e) => setKey(e.target.value)}
        style={{ font: "inherit", fontSize: 13, padding: "4px 8px", borderRadius: 6, border: "1px solid var(--border-strong)", width: 200 }}
      />
      <Button
        small
        disabled={!key.trim() || m.setApiKey.isPending}
        onClick={() => m.setApiKey.mutate({ id: p.id, key: key.trim() }, { onSuccess: () => setKey("") })}
      >
        Save key
      </Button>
      {m.setApiKey.error && (
        <span role="alert" style={{ fontSize: 12, color: "var(--warn-strong)" }}>
          {errorMessage(m.setApiKey.error)}
        </span>
      )}
    </div>
  );
}

function PriceHistory() {
  const { data } = useBackfillStatus();
  if (!data) return null;
  return (
    <Card title="Price history" label="Price history">
      <ul style={{ margin: 0, padding: 0, listStyle: "none" }}>
        {data.sources.map((s) => (
          <li key={s.id} data-testid="history-row" style={{ padding: "8px 0", borderTop: "1px solid var(--divider)", display: "flex", gap: 10, alignItems: "baseline", flexWrap: "wrap" }}>
            <strong style={{ minWidth: 60 }}>{s.name}</strong>
            <Badge tone={HISTORY[s.state]?.tone ?? "neutral"}>{HISTORY[s.state]?.label ?? s.state}</Badge>
            <span style={{ fontSize: 13, color: s.state === "failed" ? "var(--warn-strong)" : "var(--text-secondary)" }}>{historyText(s)}</span>
            {s.note && <span style={{ fontSize: 12, color: "var(--text-muted)" }}>{s.note}</span>}
          </li>
        ))}
      </ul>
      <span style={{ fontSize: 12, color: "var(--text-muted)" }}>
        Past prices and rates are downloaded once after a refresh and kept on this computer
        {data.cache_path ? (
          <>
            {" "}
            in <code>{data.cache_path}</code>
          </>
        ) : null}
        . The cache holds public market data only; deleting it is safe.
      </span>
    </Card>
  );
}

export function Settings() {
  const { data } = useSettings();
  const m = useSettingsMutations();
  if (!data) return <PageHeader title="Settings" />;
  return (
    <>
      <PageHeader title="Settings" />
      <Card title="Valuation">
        <label style={{ display: "flex", alignItems: "center", gap: 10 }}>
          <span>Base currency</span>
          <select
            value={data.base_currency}
            onChange={(e) => m.setBaseCurrency.mutate(e.target.value)}
            style={{ font: "inherit", padding: "5px 8px", borderRadius: 6, border: "1px solid var(--border-strong)" }}
          >
            {data.currencies.map((c) => (
              <option key={c} value={c}>
                {c}
              </option>
            ))}
          </select>
        </label>
        <span style={{ fontSize: 12, color: "var(--text-muted)" }}>Totals are converted with TCMB forex buying rates through TRY.</span>
      </Card>
      <Card title="Market data providers" aside={<RefreshButton />}>
        <DataTable label="Market data providers" minWidth={560}>
          <thead>
            <tr>
              <th scope="col">Provider</th>
              <th scope="col">Supplies</th>
              <th scope="col">Last successful refresh</th>
              <th scope="col" style={{ textAlign: "right" }}>
                Enabled
              </th>
            </tr>
          </thead>
          <tbody>
            {data.providers.map((p) => (
              <tr key={p.id} data-testid="provider-row">
                <td style={{ fontWeight: 500 }}>{p.name}</td>
                <td style={{ color: "var(--text-secondary)" }}>
                  {p.supplies}
                  {p.needs_api_key && <ApiKeyField p={p} />}
                </td>
                <td className="num">{when(p.last_success)}</td>
                <td style={{ textAlign: "right" }}>
                  <label style={{ display: "inline-flex", gap: 6, alignItems: "center" }}>
                    <input
                      type="checkbox"
                      role="switch"
                      checked={p.enabled}
                      aria-label={`${p.name} enabled`}
                      onChange={(e) => m.setProviderEnabled.mutate({ id: p.id, enabled: e.target.checked })}
                    />
                    {p.enabled ? "On" : "Off"}
                  </label>
                </td>
              </tr>
            ))}
          </tbody>
        </DataTable>
        <label style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <input type="checkbox" checked={data.refresh_on_start} onChange={(e) => m.setRefreshOnStart.mutate(e.target.checked)} />
          Refresh prices when the app starts, if the last refresh is older than a day
        </label>
        <span style={{ fontSize: 12, color: "var(--text-muted)" }}>
          A disabled provider makes no network requests; its stored data is still used. Stock and ETF providers are asked only for the assets you
          assign to them on the Assets screen. Last refresh: {when(data.last_refresh_at)}.
        </span>
        {data.last_report && <Report report={data.last_report} />}
      </Card>
      <PriceHistory />
    </>
  );
}
