import { useMutation } from "@tanstack/react-query";
import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useRef, type ReactNode } from "react";
import { Link, useLocation } from "react-router";
import type { BackupRequest } from "../app/useMenuEvents";
import { PageHeader } from "../app/Layout";
import { PERIODS, useScopeName, useView } from "../app/view";
import { api, errorMessage } from "../bridge/api";
import { Button, Card } from "../ui";

const EXPORTS_VIEW = { scope: true, period: true } as const;
const muted = { fontSize: 13, color: "var(--text-muted)" } as const;

/** One export: a title, what it contains, and a button that asks for a folder. */
function ExportRow({ title, what, label, run, start }: { title: string; what: ReactNode; label: string; run: (dir: string) => Promise<string>; start?: number }) {
  const m = useMutation({ mutationFn: run });
  const go = async () => {
    const dir = await open({ directory: true, title: `Choose a folder for ${title.toLowerCase()}` });
    if (typeof dir === "string") m.mutate(dir);
  };
  // File › Back up database… arrives with a start token.
  const started = useRef<number | undefined>(undefined);
  useEffect(() => {
    if (start && started.current !== start) {
      started.current = start;
      void go();
    }
  }, [start]); // eslint-disable-line react-hooks/exhaustive-deps
  return (
    <div style={{ display: "flex", flexWrap: "wrap", gap: 12, alignItems: "center", padding: "14px 0", borderTop: "1px solid var(--divider)" }} data-testid="export-row">
      <div style={{ flex: "1 1 320px", minWidth: 0, display: "flex", flexDirection: "column", gap: 2 }}>
        <span style={{ fontWeight: 500 }}>{title}</span>
        <span style={muted}>{what}</span>
        {m.data && (
          <span style={{ fontSize: 12, color: "var(--gain)" }} role="status">
            Saved {m.data}
          </span>
        )}
        {m.error && (
          <span role="alert" style={{ fontSize: 12, color: "var(--warn-strong)" }}>
            {errorMessage(m.error)}
          </span>
        )}
      </div>
      <Button onClick={() => void go()} disabled={m.isPending}>
        {label}
      </Button>
    </div>
  );
}

export function Exports() {
  const backupToken = (useLocation().state as BackupRequest | null)?.backup;
  const { view } = useView();
  const scopeName = useScopeName();
  const periodLabel = PERIODS.find((p) => p.value === view.period)?.label ?? "";
  return (
    <>
      <PageHeader title="Exports" subtitle="Take your data out of Mizan. Everything is written to a folder you choose." view={EXPORTS_VIEW} />
      <Card title="Backup" label="Backup">
        <ExportRow
          title="Database backup"
          what="A complete copy of your Mizan database, named mizan-backup-<date-time>.db. Open it with Mizan or any SQLite tool."
          label="Back up database…"
          run={api.backupDatabase}
          start={backupToken}
        />
      </Card>
      <Card title="CSV files" label="CSV files">
        <ExportRow title="Activities CSV" what="Every stored activity, imported and manual, with its source." label="Export…" run={api.exportActivitiesCsv} />
        <ExportRow
          title="Holdings CSV"
          what={`Positions and cash as of today with values in the base currency · ${scopeName}`}
          label="Export…"
          run={(dir) => api.exportHoldingsCsv(view.account_id, dir)}
        />
        <ExportRow
          title="Income CSV"
          what={`One row per dividend, distribution and interest payment · ${scopeName} · ${periodLabel}`}
          label="Export…"
          run={(dir) => api.exportIncomeCsv(view.period, view.account_id, dir)}
        />
      </Card>
      <Card title="Made elsewhere" label="Other exports">
        <div style={{ display: "flex", flexDirection: "column" }}>
          <div style={{ padding: "12px 0", borderTop: "1px solid var(--divider)" }}>
            <Link to="/tax" style={{ fontWeight: 500 }}>
              Tax report CSV
            </Link>
            <div style={muted}>Gains and income for a tax year, from the Tax screen.</div>
          </div>
          <div style={{ padding: "12px 0", borderTop: "1px solid var(--divider)" }}>
            <Link to="/market-data" style={{ fontWeight: 500 }}>
              Market data series CSV
            </Link>
            <div style={muted}>Any price, rate or index series with its history, from Market data.</div>
          </div>
        </div>
      </Card>
    </>
  );
}
