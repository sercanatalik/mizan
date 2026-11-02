import type { BackfillProgress } from "../bridge/generated/BackfillProgress";
import { useBackfillStatus, useValueHistory } from "../bridge/queries";
import { Card, EmptyState, LineChart } from "../ui";
import { longDate } from "./format";
import { historyText } from "./Settings";

/** "TEFAS 34 of 120 months · TCMB 200 of 1250 days" while downloading, otherwise null. */
export function downloading(p: BackfillProgress | undefined): string | null {
  const running = p?.sources.filter((s) => s.state === "running") ?? [];
  return running.length ? running.map((s) => `${s.name} ${historyText(s)}`).join(" · ") : null;
}

function waitingText(p: BackfillProgress | undefined): string {
  const parts = p?.sources.filter((s) => s.state !== "idle").map((s) => `${s.name}: ${historyText(s)}`) ?? [];
  return parts.length ? parts.join(" · ") : "Refresh prices to start downloading price history.";
}

const compact = (v: number) =>
  Math.abs(v) >= 10_000 ? `${Math.round(v / 1000).toLocaleString("en-US")}K` : Math.round(v).toLocaleString("en-US");

export function ValueOverTime() {
  const { data } = useValueHistory();
  const { data: backfill } = useBackfillStatus();
  if (!data || data.points.length === 0) return null;
  const busy = downloading(backfill);
  return (
    <Card
      title="Value over time"
      label="Value chart"
      aside={<span style={{ fontSize: 12, color: "var(--text-muted)" }}>{data.base_currency} · daily</span>}
    >
      {data.valued_days === 0 ? (
        <EmptyState
          title="Price history needed"
          message={
            <>
              The value history appears once price history has been downloaded.
              <br />
              {waitingText(backfill)}
            </>
          }
        />
      ) : (
        <>
          <LineChart
            label="Portfolio value and net contributions"
            dates={data.points.map((p) => p.date)}
            series={[
              { name: "Value", values: data.points.map((p) => (p.value === null ? null : Number(p.value))), labels: data.points.map((p) => (p.value === null ? null : `${p.value} ${data.base_currency}`)) },
              { name: "Net contributions", values: data.points.map((p) => Number(p.contributions)), labels: data.points.map((p) => `${p.contributions} ${data.base_currency}`) },
            ]}
            estimated={data.points.map((p) => p.estimated)}
            format={compact}
            formatDate={longDate}
          />
          {busy && (
            <span role="note" style={{ fontSize: 12, color: "var(--text-muted)" }}>
              Price history is still downloading ({busy}); days without prices yet are left out.
            </span>
          )}
        </>
      )}
    </Card>
  );
}
