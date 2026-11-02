import { useQuery } from "@tanstack/react-query";
import { Link } from "react-router";
import { keys, useBackfillStatus, useSettings } from "../bridge/queries";
import type { BackfillProgress } from "../bridge/generated/BackfillProgress";
import { longDate } from "../screens/format";
import s from "./layout.module.css";

/** Percent done of the sources still downloading history, or null when none is. */
export function backfillPercent(p: BackfillProgress | undefined): number | null {
  const active = (p?.sources ?? []).filter((x) => (x.state === "running" || x.state === "waiting") && x.planned > 0);
  if (!active.some((x) => x.state === "running")) return null;
  const planned = active.reduce((n, x) => n + x.planned, 0);
  const done = active.reduce((n, x) => n + Math.min(x.done, x.planned), 0);
  return Math.floor((done / planned) * 100);
}

/** `2026-11-01T10:00:00Z` → "10:00" today in local time, otherwise "1 Nov 2026". */
export function refreshedWhen(iso: string, now = new Date()): string {
  const t = new Date(iso);
  if (Number.isNaN(t.getTime())) return iso;
  const sameDay = t.getFullYear() === now.getFullYear() && t.getMonth() === now.getMonth() && t.getDate() === now.getDate();
  if (sameDay) return `${String(t.getHours()).padStart(2, "0")}:${String(t.getMinutes()).padStart(2, "0")}`;
  const local = `${t.getFullYear()}-${String(t.getMonth() + 1).padStart(2, "0")}-${String(t.getDate()).padStart(2, "0")}`;
  return longDate(local);
}

/** Price refresh and history download state at the foot of the sidebar. */
export function DataStatus() {
  const settings = useSettings().data;
  const background = useQuery({ queryKey: keys.refreshing, queryFn: () => false, staleTime: Infinity }).data ?? false;
  const pct = backfillPercent(useBackfillStatus().data);
  const refreshing = background || settings?.refreshing === true;
  const line = refreshing
    ? "Refreshing prices…"
    : settings?.last_refresh_at
      ? `Prices refreshed ${refreshedWhen(settings.last_refresh_at)}`
      : "Prices not refreshed yet";
  return (
    <Link to="/market-data" className={s.status} aria-label={`Data status: ${line}${pct !== null ? `, downloading history ${pct}%` : ""}`}>
      <span className={s.statusLine}>
        <span className={refreshing ? s.dotBusy : settings?.last_refresh_at ? s.dotOk : s.dotIdle} aria-hidden="true" />
        {line}
      </span>
      {pct !== null && (
        <span className={s.statusLine}>
          <span className={s.statusText}>Downloading history · {pct}%</span>
          <span className={s.bar} role="progressbar" aria-label="History download" aria-valuemin={0} aria-valuemax={100} aria-valuenow={pct}>
            <span className={s.barFill} style={{ width: `${pct}%` }} />
          </span>
        </span>
      )}
    </Link>
  );
}
