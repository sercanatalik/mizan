import { Link } from "react-router";
import { PageHeader } from "../app/Layout";
import type { AccountStatus } from "../bridge/generated/AccountStatus";
import type { AttentionItem } from "../bridge/generated/AttentionItem";
import { useOverview } from "../bridge/queries";
import { Card, EmptyState } from "../ui";
import { longDate } from "./format";
import { NoData } from "./NoData";
import { RefreshButton } from "./RefreshButton";
import { ValueOverTime } from "./ValueOverTime";
import type { TotalView } from "../bridge/generated/TotalView";

function excluded(t: TotalView): string | null {
  const parts = [];
  if (t.excluded_no_price) parts.push(`${t.excluded_no_price} position${t.excluded_no_price === 1 ? "" : "s"} without a price`);
  if (t.excluded_no_rate) parts.push(`${t.excluded_no_rate} without an exchange rate`);
  return parts.length ? `excludes ${parts.join(" and ")}` : null;
}

function statusText(a: AccountStatus): { text: string; warn: boolean } {
  const c = a.latest_check;
  if (!c) return { text: "No balance check yet", warn: false };
  if (c.passed) return { text: `Statement to ${longDate(c.as_of)} · all balances match`, warn: false };
  const labels = [...new Set(c.failed.map((f) => f.display))].join(", ");
  const n = c.failed.length;
  return { text: `${n} balance check${n === 1 ? "" : "s"} failed · ${labels}`, warn: true };
}

function attention(item: AttentionItem): { title: string; detail?: string; to: string; action: string; warn: boolean } {
  switch (item.type) {
    case "pending_decisions":
      return { title: `${item.count} record${item.count === 1 ? "" : "s"} need your decision`, to: "/imports", action: "Review", warn: false };
    case "failed_checks":
      return {
        title: `${item.account_name} balance checks failed`,
        detail: item.failed.map((f) => `${f.display}: statement ${f.expected}, Mizan ${f.computed}`).join(" · ") + ` (as of ${longDate(item.as_of)})`,
        to: "/imports",
        action: "Investigate",
        warn: true,
      };
    case "unknown_basis":
      return { title: `Unknown cost basis · ${item.units} ${item.display} in ${item.account_name}`, to: "/holdings", action: "View", warn: false };
  }
}

const row = { display: "flex", alignItems: "flex-start", gap: 12, padding: "12px 0", borderTop: "1px solid var(--divider)" } as const;

export function Overview() {
  const { data } = useOverview();
  if (!data) return <PageHeader title="Overview" />;
  if (data.accounts.length === 0) {
    return (
      <>
        <PageHeader title="Overview" />
        <NoData what="your accounts" />
      </>
    );
  }
  return (
    <>
      <PageHeader title="Overview" subtitle="All accounts" actions={<RefreshButton />} />
      <Card title="Portfolio value" label="Portfolio value">
        {data.has_prices ? (
          <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
            <span className="num" style={{ fontSize: 30, fontWeight: 500, letterSpacing: "-0.02em" }} data-testid="portfolio-total">
              {data.total.amount} {data.total.currency}
            </span>
            <span style={{ fontSize: 12, color: "var(--text-muted)" }}>
              {data.total.oldest_price ? `Prices as of ${longDate(data.total.oldest_price)} or later` : null}
              {excluded(data.total) ? ` · ${excluded(data.total)}` : ""}
            </span>
          </div>
        ) : (
          <EmptyState title="Prices needed" message="Refresh prices to see the portfolio value. Assign price sources on the Assets screen." action={{ to: "/assets", label: "Go to Assets" }} />
        )}
      </Card>
      <ValueOverTime />
      <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(min(420px, 100%), 1fr))", gap: 14, alignItems: "start" }}>
        <Card title="Accounts" label="Accounts">
          <div>
            {data.accounts.map((a) => {
              const st = statusText(a);
              return (
                <div key={a.account_id} style={row} data-testid="account-row">
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <div style={{ fontWeight: 500 }}>{a.name}</div>
                    <div style={{ fontSize: 12, color: st.warn ? "var(--warn-strong)" : "var(--text-muted)" }}>
                      {st.warn && (
                        <svg role="img" aria-label="Warning" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round" style={{ verticalAlign: "-2px", marginRight: 4 }}>
                          <path d="M12 9v4M12 17h.01M10.3 3.9L1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0z" />
                        </svg>
                      )}
                      {st.text}
                    </div>
                  </div>
                  <span style={{ display: "flex", flexDirection: "column", alignItems: "flex-end", gap: 2 }}>
                    {data.has_prices && a.total && (
                      <span className="num" style={{ fontWeight: 500 }} data-testid="account-total">
                        {a.total.amount} {a.total.currency}
                      </span>
                    )}
                    <span style={{ fontSize: 12, color: "var(--text-muted)" }}>
                      {a.activity_count} activit{a.activity_count === 1 ? "y" : "ies"}
                    </span>
                  </span>
                </div>
              );
            })}
          </div>
        </Card>
        <Card title="Needs attention" label="Needs attention">
          {data.attention.length === 0 ? (
            <p style={{ margin: 0, color: "var(--text-muted)" }}>Everything is in order.</p>
          ) : (
            <div>
              {data.attention.map((item, i) => {
                const a = attention(item);
                return (
                  <div key={i} style={row} data-testid="attention-item">
                    <span aria-hidden="true" style={{ marginTop: 6, width: 8, height: 8, borderRadius: 999, flex: "none", background: a.warn ? "var(--warn)" : "var(--text-muted)" }} />
                    <div style={{ flex: 1, minWidth: 0 }}>
                      <div style={{ fontWeight: 500 }}>{a.title}</div>
                      {a.detail && <div style={{ fontSize: 12, color: "var(--text-muted)" }}>{a.detail}</div>}
                    </div>
                    <Link to={a.to} style={{ fontSize: 13, fontWeight: 500, textDecoration: "none", whiteSpace: "nowrap" }}>
                      {a.action}
                    </Link>
                  </div>
                );
              })}
            </div>
          )}
        </Card>
      </div>
    </>
  );
}
