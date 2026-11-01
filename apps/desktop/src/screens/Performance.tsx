import { useState } from "react";
import { PageHeader } from "../app/Layout";
import type { Period } from "../bridge/generated/Period";
import { useContributions } from "../bridge/queries";
import { Card, DataTable, EmptyState, Segmented } from "../ui";
import { longDate } from "./format";
import { NoData } from "./NoData";

const RETURN_CARDS = ["Time-weighted return", "Money-weighted return", "Benchmark"];

export function Performance() {
  const { data } = useContributions();
  const [period, setPeriod] = useState<Period>("ytd");
  if (!data) return <PageHeader title="Performance" />;
  const current = data.periods.find((p) => p.period === period);
  if ((data.periods[0]?.accounts.length ?? 0) === 0 || !current) {
    return (
      <>
        <PageHeader title="Performance" />
        <NoData what="contributions" />
      </>
    );
  }
  return (
    <>
      <PageHeader
        title="Performance"
        subtitle={`All accounts · to ${longDate(current.to)}`}
        actions={
          <Segmented
            label="Period"
            value={period}
            onChange={setPeriod}
            options={[
              { value: "ytd", label: "YTD" },
              { value: "last12_months", label: "12M" },
              { value: "since_start", label: "Since start" },
            ]}
          />
        }
      />
      <section aria-label="Returns" style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(min(220px, 100%), 1fr))", gap: 14 }}>
        {RETURN_CARDS.map((title) => (
          <Card key={title} title={title}>
            <span style={{ color: "var(--text-muted)", fontSize: 13 }}>Needs the returns engine and market data.</span>
          </Card>
        ))}
      </section>
      <Card title="Growth" label="Growth chart">
        <EmptyState title="Returns engine needed" message="The growth chart and benchmark comparison appear once the returns engine and market data are available." />
      </Card>
      <Card title="Net contributions" label="Net contributions" aside={<span style={{ fontSize: 12, color: "var(--text-muted)" }}>Deposits minus withdrawals; internal transfers excluded</span>}>
        <DataTable label="Net contributions" minWidth={520}>
          <thead>
            <tr>
              <th scope="col">Account</th>
              <th scope="col">From</th>
              <th scope="col" style={{ textAlign: "right" }}>Net contribution</th>
            </tr>
          </thead>
          <tbody>
            {current.accounts.map((a) => (
              <tr key={a.account_id} data-testid="contribution-row">
                <td style={{ fontWeight: 500 }}>{a.name}</td>
                <td className="num">{a.from ?? "—"}</td>
                <td className="num" style={{ textAlign: "right" }}>
                  {a.by_currency.length ? a.by_currency.map((c) => `${c.amount} ${c.currency}`).join(" · ") : "—"}
                </td>
              </tr>
            ))}
            <tr data-testid="contribution-total">
              <td style={{ fontWeight: 600 }}>Total</td>
              <td />
              <td className="num" style={{ textAlign: "right", fontWeight: 600 }}>
                {current.total.length ? current.total.map((c) => `${c.amount} ${c.currency}`).join(" · ") : "—"}
              </td>
            </tr>
          </tbody>
        </DataTable>
      </Card>
    </>
  );
}
