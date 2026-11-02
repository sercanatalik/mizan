import { useState } from "react";
import { useLocation, useNavigate } from "react-router";
import { PageHeader } from "../app/Layout";
import type { AddActivityRequest, EditAssetRequest } from "../app/ViewBar";
import { errorMessage } from "../bridge/api";
import type { InboxItem } from "../bridge/generated/InboxItem";
import type { InboxKind } from "../bridge/generated/InboxKind";
import { useInbox, useInboxMutations } from "../bridge/queries";
import { Button, Card, EmptyState, Segmented } from "../ui";
import { amount, longDate } from "./format";
import s from "./inbox.module.css";

type Filter = "all" | InboxKind;

const KINDS: { value: InboxKind; label: string; detail: string }[] = [
  { value: "import", label: "Imports", detail: "Import decision" },
  { value: "check", label: "Balance checks", detail: "Balance check" },
  { value: "price", label: "Market data", detail: "Market data" },
  { value: "cost", label: "Cost basis", detail: "Cost basis" },
];

const ICONS: Record<InboxKind, string> = {
  import: "M8 8h12v12H8zM4 16V4h12",
  check: "M12 8v5M12 16h.01M10.3 3.9L2 18a2 2 0 0 0 1.7 3h16.6a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0z",
  price: "M12 7v5l3 2M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18z",
  cost: "M12 3v18M17 7H9.5a3 3 0 0 0 0 6h5a3 3 0 0 1 0 6H6",
};

/** Location state asking the Inbox to select one item (from the Overview preview). */
export interface InboxSelectRequest {
  select: string;
}

function KindIcon({ kind }: { kind: InboxKind }) {
  return (
    <span className={`${s.icon} ${s[`icon-${kind}`]}`} aria-hidden="true">
      <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <path d={ICONS[kind]} />
      </svg>
    </span>
  );
}

/** The primary action of an item, and an optional second one. */
function useActions(item: InboxItem) {
  const navigate = useNavigate();
  const a = item.action;
  switch (a.type) {
    case "import_review":
      return { primary: { label: "Open the review", run: () => navigate("/imports") }, secondary: null };
    case "account_checks":
      return {
        primary: { label: "Open balance checks", run: () => navigate(`/accounts/${a.account_id}#balance-checks`) },
        secondary: { label: "Open activity", run: () => navigate(`/activity?account=${a.account_id}`) },
      };
    case "add_position_in":
      return {
        primary: {
          label: "Record the cost",
          run: () =>
            navigate("/activity", {
              state: { add: { accountId: a.account_id, at: Date.now(), kind: "position_in", assetId: a.asset_id } } satisfies AddActivityRequest,
            }),
        },
        secondary: null,
      };
    case "asset_price":
      return {
        primary: {
          label: "Set the price",
          run: () => (a.asset_id !== null ? navigate("/assets", { state: { editAsset: a.asset_id } satisfies EditAssetRequest }) : navigate("/assets")),
        },
        secondary: { label: "Open market data", run: () => navigate("/market-data") },
      };
  }
}

function Detail({ item }: { item: InboxItem }) {
  const { primary, secondary } = useActions(item);
  const m = useInboxMutations();
  const kind = KINDS.find((k) => k.value === item.kind);
  return (
    <Card label="Item detail">
      <div className={s.detail}>
        <div className={s.kindRow}>
          <KindIcon kind={item.kind} />
          <span className={s.kindLabel}>{kind?.detail}</span>
        </div>
        <h2 className={s.detailTitle}>{item.title}</h2>
        <p className={s.body}>{item.body}</p>
        {item.check_lines.length > 0 && (
          <table className={s.lines} aria-label="Failing lines">
            <thead>
              <tr>
                <th scope="col">Line</th>
                <th scope="col">Statement</th>
                <th scope="col">Mizan computes</th>
                <th scope="col">Difference</th>
              </tr>
            </thead>
            <tbody>
              {item.check_lines.map((l) => (
                <tr key={l.label}>
                  <th scope="row">{l.display === "cash" ? `${l.label} cash` : l.display}</th>
                  <td className="num">{amount(l.expected)}</td>
                  <td className="num">{amount(l.computed)}</td>
                  <td className="num">{l.difference ? amount(l.difference) : "—"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
        {item.facts.length > 0 && (
          <dl className={s.facts}>
            {item.facts.map((f, i) => (
              <div key={`${f.label}-${i}`} className={s.fact}>
                <dt>{f.label}</dt>
                <dd>{f.value}</dd>
              </div>
            ))}
          </dl>
        )}
        <div className={s.actions}>
          <Button variant="primary" onClick={primary.run}>
            {primary.label}
          </Button>
          {secondary && <Button onClick={secondary.run}>{secondary.label}</Button>}
          <Button variant="ghost" onClick={() => m.snooze.mutate({ key: item.key, days: 1 })} disabled={m.snooze.isPending}>
            Snooze 1 day
          </Button>
          <Button variant="ghost" onClick={() => m.snooze.mutate({ key: item.key, days: 7 })} disabled={m.snooze.isPending}>
            Snooze 7 days
          </Button>
        </div>
        {m.snooze.error && (
          <span role="alert" className={s.error}>
            {errorMessage(m.snooze.error)}
          </span>
        )}
      </div>
    </Card>
  );
}

function untilText(iso: string | null): string {
  return iso ? `until ${longDate(iso.slice(0, 10))}` : "";
}

export function Inbox() {
  const { data } = useInbox();
  const request = (useLocation().state as InboxSelectRequest | null)?.select ?? null;
  const [filter, setFilter] = useState<Filter>("all");
  const [selected, setSelected] = useState<string | null>(request);
  const m = useInboxMutations();
  const header = <PageHeader title="Inbox" subtitle="Everything that needs a decision before the numbers can be trusted" />;
  if (!data) return header;

  const items = data.items;
  const shown = items.filter((i) => filter === "all" || i.kind === filter);
  const current = shown.find((i) => i.key === selected) ?? shown[0] ?? null;
  const options = [
    { value: "all" as Filter, label: "All", count: items.length },
    ...KINDS.map((k) => ({ value: k.value as Filter, label: k.label, count: items.filter((i) => i.kind === k.value).length })),
  ];

  return (
    <>
      {header}
      {items.length === 0 ? (
        <EmptyState title="Everything is in order" message="Every figure is backed by checked records. New items appear here when an import, a balance check or a price needs you." />
      ) : (
        <>
          <Segmented label="Filter" value={filter} onChange={setFilter} options={options} />
          <div className={s.split}>
            <Card label="Items">
              <ul className={s.list}>
                {shown.map((i) => (
                  <li key={i.key}>
                    <button type="button" className={s.row} aria-current={current?.key === i.key ? "true" : undefined} onClick={() => setSelected(i.key)}>
                      <KindIcon kind={i.kind} />
                      <span className={s.rowText}>
                        <span className={s.rowTitle}>{i.title}</span>
                        <span className={s.rowSub}>{i.subtitle}</span>
                      </span>
                      {i.since && <span className={s.rowDate}>{longDate(i.since)}</span>}
                    </button>
                  </li>
                ))}
              </ul>
              {shown.length === 0 && <p className={s.none}>Nothing of this type.</p>}
            </Card>
            {current && <Detail key={current.key} item={current} />}
          </div>
        </>
      )}
      {data.snoozed.length > 0 && (
        <Card title="Snoozed" label="Snoozed">
          <ul className={s.list}>
            {data.snoozed.map((i) => (
              <li key={i.key} className={s.snoozedRow}>
                <KindIcon kind={i.kind} />
                <span className={s.rowText}>
                  <span className={s.rowTitle}>{i.title}</span>
                  <span className={s.rowSub}>{untilText(i.snoozed_until)}</span>
                </span>
                <Button small onClick={() => m.unsnooze.mutate(i.key)}>
                  Unsnooze
                </Button>
              </li>
            ))}
          </ul>
        </Card>
      )}
    </>
  );
}
