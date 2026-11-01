import { useMemo, useState } from "react";
import { Link } from "react-router";
import { PageHeader } from "../app/Layout";
import type { CostView } from "../bridge/generated/CostView";
import type { Holdings as HoldingsData } from "../bridge/generated/Holdings";
import type { Position } from "../bridge/generated/Position";
import { useHoldings } from "../bridge/queries";
import { Badge, Card, DataTable, Segmented } from "../ui";
import { NoData } from "./NoData";

type GroupBy = "account" | "class" | "currency";

const CLASS_LABELS: Record<string, string> = {
  equity: "Equities",
  etf: "ETFs",
  fund: "Funds",
  bond: "Bonds",
  option: "Options",
  crypto: "Crypto",
  commodity: "Commodities",
  cash: "Cash",
  other: "Other",
};

interface Row {
  key: string;
  account: string;
  label: string;
  sub: string | null;
  quantity: string;
  position?: Position;
  cashCurrency?: string;
}

function costNote(c: CostView): string | null {
  switch (c.state) {
    case "known":
      return null;
    case "unknown_basis":
      return `not available: ${c.unknown_units ?? "some"} units with unknown cost basis`;
    case "mixed_currency":
      return "not available: paid in more than one currency";
    case "short":
      return "not available for short positions yet";
  }
}

function rowsOf(data: HoldingsData): Row[] {
  const rows: Row[] = [];
  for (const a of data.accounts) {
    for (const p of a.positions) {
      rows.push({
        key: `${a.account_id}-${p.instrument_key}`,
        account: a.name,
        label: p.symbol ?? p.name ?? p.instrument_key,
        sub: p.symbol && p.name ? p.name : p.isin ?? null,
        quantity: p.quantity,
        position: p,
      });
    }
    for (const c of a.cash) {
      rows.push({ key: `${a.account_id}-cash-${c.currency}`, account: a.name, label: `Cash · ${c.currency}`, sub: null, quantity: c.amount, cashCurrency: c.currency });
    }
  }
  return rows;
}

function groupName(r: Row, by: GroupBy): string {
  if (by === "account") return r.account;
  if (by === "class") return r.cashCurrency ? "Cash" : CLASS_LABELS[r.position?.class ?? "other"] ?? "Other";
  return r.cashCurrency ?? r.position?.currency ?? "Unknown currency";
}

export function Holdings() {
  const { data } = useHoldings();
  const [by, setBy] = useState<GroupBy>("account");
  const rows = useMemo(() => (data ? rowsOf(data) : []), [data]);
  const groups = useMemo(() => {
    const m = new Map<string, Row[]>();
    for (const r of rows) m.set(groupName(r, by), [...(m.get(groupName(r, by)) ?? []), r]);
    return [...m.entries()];
  }, [rows, by]);

  if (!data) return <PageHeader title="Holdings" />;
  if (rows.length === 0) {
    return (
      <>
        <PageHeader title="Holdings" />
        <NoData what="your positions" />
      </>
    );
  }
  const positions = rows.filter((r) => r.position).length;
  return (
    <>
      <PageHeader
        title="Holdings"
        subtitle={`${positions} position${positions === 1 ? "" : "s"} in ${data.accounts.length} account${data.accounts.length === 1 ? "" : "s"}`}
        actions={
          <Segmented
            label="Group by"
            value={by}
            onChange={setBy}
            options={[
              { value: "account", label: "Account" },
              { value: "class", label: "Asset class" },
              { value: "currency", label: "Currency" },
            ]}
          />
        }
      />
      <div role="note" style={{ color: "var(--text-muted)", fontSize: 13 }}>
        Market value, weights and gains appear once market data is available. Costs are provisional averages in the currency paid.
      </div>
      <Card label="Positions">
        <DataTable label="Positions" minWidth={820}>
          <thead>
            <tr>
              <th scope="col">Instrument</th>
              <th scope="col">Account</th>
              <th scope="col" style={{ textAlign: "right" }}>Quantity</th>
              <th scope="col" style={{ textAlign: "right" }}>Average cost (provisional)</th>
              <th scope="col" style={{ textAlign: "right" }}>Total cost (provisional)</th>
            </tr>
          </thead>
          {groups.map(([name, items]) => (
            <tbody key={name} data-testid="holdings-group">
              <tr>
                <th scope="rowgroup" colSpan={5} style={{ background: "var(--surface-sunken)", color: "var(--text)", fontSize: 13, fontWeight: 600 }}>
                  {name} <span style={{ fontWeight: 400, color: "var(--text-muted)" }}>· {items.length}</span>
                </th>
              </tr>
              {items.map((r) => {
                const p = r.position;
                const note = p ? costNote(p.cost) : null;
                return (
                  <tr key={r.key} data-testid="holding-row">
                    <td>
                      <div style={{ fontWeight: 500 }}>{r.label}</div>
                      {r.sub && <div style={{ fontSize: 12, color: "var(--text-muted)" }}>{r.sub}</div>}
                      <div style={{ display: "flex", flexWrap: "wrap", gap: 6, marginTop: 4 }}>
                        {p?.cost.unknown_units && <Badge>{p.cost.unknown_units} units with unknown cost basis</Badge>}
                        {p?.check_flag && (
                          <Link to="/imports" style={{ textDecoration: "none" }} aria-label={`Statement says ${p.check_flag.statement_quantity}, see the import`}>
                            <Badge tone="warn">Statement says {p.check_flag.statement_quantity}</Badge>
                          </Link>
                        )}
                      </div>
                    </td>
                    <td style={{ color: "var(--text-secondary)" }}>{r.account}</td>
                    <td className="num" style={{ textAlign: "right" }}>
                      {r.quantity}
                    </td>
                    {r.cashCurrency ? (
                      <td colSpan={2} />
                    ) : note ? (
                      <td colSpan={2} style={{ textAlign: "right", color: "var(--text-muted)", fontSize: 12 }}>
                        {note}
                      </td>
                    ) : (
                      <>
                        <td className="num" style={{ textAlign: "right" }}>
                          {p?.cost.average} {p?.cost.currency}
                        </td>
                        <td className="num" style={{ textAlign: "right" }}>
                          {p?.cost.total} {p?.cost.currency}
                        </td>
                      </>
                    )}
                  </tr>
                );
              })}
            </tbody>
          ))}
        </DataTable>
      </Card>
    </>
  );
}
