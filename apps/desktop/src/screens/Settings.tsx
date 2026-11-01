import { PageHeader } from "../app/Layout";
import type { RefreshReport } from "../bridge/generated/RefreshReport";
import { useSettings, useSettingsMutations } from "../bridge/queries";
import { Badge, Card, DataTable } from "../ui";
import { RefreshButton } from "./RefreshButton";

const STATUS: Record<string, { label: string; tone: "accent" | "warn" | "neutral" }> = {
  ok: { label: "OK", tone: "accent" },
  failed: { label: "Failed", tone: "warn" },
  disabled: { label: "Disabled", tone: "neutral" },
  idle: { label: "Nothing to fetch", tone: "neutral" },
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
                <td style={{ color: "var(--text-secondary)" }}>{p.supplies}</td>
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
          A disabled provider makes no network requests; its stored data is still used. Last refresh: {when(data.last_refresh_at)}.
        </span>
        {data.last_report && <Report report={data.last_report} />}
      </Card>
    </>
  );
}
