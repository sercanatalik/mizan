import { useState } from "react";
import { Link } from "react-router";
import { PageHeader } from "../app/Layout";
import { errorMessage, type ResetScopeName } from "../bridge/api";
import { useResetData, useResetPreview, useSettings, useSettingsMutations } from "../bridge/queries";
import { Button, Card } from "../ui";

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
      <Card title="Price providers" label="Price providers">
        <p style={{ margin: 0 }}>
          Price providers, API keys, refresh on start and the inflation index moved to{" "}
          <Link to="/market-data?tab=providers">Market data › Providers</Link>.
        </p>
      </Card>
      <DangerZone />
    </>
  );
}

const SCOPES: { value: ResetScopeName; label: string; clears: string; keeps: string }[] = [
  {
    value: "portfolio",
    label: "Portfolio data",
    clears: "Accounts, assets, imports, activities, balance checks, manual and edited values, and prices stored in this database.",
    keeps: "Settings, API keys, saved column mappings and the downloaded market history.",
  },
  {
    value: "everything",
    label: "Everything",
    clears: "All of the above, settings and API keys (back to defaults), and the shared market history, which is downloaded again on the next refresh.",
    keeps: "Saved column mappings.",
  },
];

function DangerZone() {
  const [scope, setScope] = useState<ResetScopeName>("portfolio");
  const [typed, setTyped] = useState("");
  const { data: preview } = useResetPreview(scope);
  const reset = useResetData();
  const n = (v: number | bigint) => Number(v).toLocaleString("en-US");
  return (
    <Card title="Danger zone" label="Reset data">
      <p style={{ margin: 0, fontSize: 13 }}>
        Reset data to start over. A backup copy is written next to each file before anything is cleared.
      </p>
      <fieldset style={{ border: 0, margin: 0, padding: 0, display: "grid", gap: 10 }}>
        <legend style={{ fontSize: 12, fontWeight: 600, color: "var(--text-secondary)", marginBottom: 6 }}>What to reset</legend>
        {SCOPES.map((o) => (
          <label key={o.value} style={{ display: "grid", gridTemplateColumns: "auto 1fr", gap: "2px 8px", fontSize: 13 }}>
            <input
              type="radio"
              name="reset-scope"
              value={o.value}
              checked={scope === o.value}
              onChange={() => {
                setScope(o.value);
                reset.reset();
              }}
            />
            <strong style={{ fontWeight: 600 }}>{o.label}</strong>
            <span />
            <span style={{ fontSize: 12, color: "var(--text-muted)" }}>
              Clears: {o.clears} Keeps: {o.keeps}
            </span>
          </label>
        ))}
      </fieldset>
      {preview && (
        <p style={{ margin: 0, fontSize: 13 }} data-testid="reset-preview">
          This clears {n(preview.accounts)} accounts, {n(preview.activities)} activities, {n(preview.imports)} imports and {n(preview.assets)} assets
          {preview.cache_values !== null && <>, and {n(preview.cache_values)} cached prices, rates and index values</>}.
          {preview.cache_values !== null && (
            <span style={{ display: "block", color: "var(--warn-strong)" }}>
              Other Mizan databases on this computer share the market history{preview.cache_path ? ` (${preview.cache_path})` : ""}; they download it again on
              their next refresh.
            </span>
          )}
        </p>
      )}
      <label style={{ display: "flex", gap: 8, alignItems: "center", fontSize: 13, flexWrap: "wrap" }}>
        Type RESET to confirm
        <input
          aria-label="Type RESET to confirm"
          value={typed}
          onChange={(e) => setTyped(e.target.value)}
          autoComplete="off"
          style={{ font: "inherit", padding: "4px 8px", borderRadius: 6, border: "1px solid var(--border-strong)", width: 120 }}
        />
        <Button
          variant="primary"
          disabled={typed !== "RESET" || reset.isPending}
          onClick={() => reset.mutate({ scope, confirm: typed }, { onSuccess: () => setTyped("") })}
          style={{ background: "var(--loss)", borderColor: "var(--loss)" }}
        >
          Reset {scope === "portfolio" ? "portfolio data" : "everything"}
        </Button>
      </label>
      {reset.error && (
        <p role="alert" style={{ margin: 0, fontSize: 12, color: "var(--loss)" }}>
          {errorMessage(reset.error)}
        </p>
      )}
      {reset.data && (
        <p role="status" style={{ margin: 0, fontSize: 13 }} data-testid="reset-done">
          Reset complete.{" "}
          {reset.data.backups.length > 0 ? (
            <>
              Backups:{" "}
              {reset.data.backups.map((b) => (
                <code key={b} style={{ display: "block" }}>
                  {b}
                </code>
              ))}
            </>
          ) : (
            "No backup was needed."
          )}
        </p>
      )}
    </Card>
  );
}
