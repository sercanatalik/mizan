import { useState } from "react";
import { Link } from "react-router";
import { PageHeader } from "../app/Layout";
import type { AccountListRow } from "../bridge/generated/AccountListRow";
import type { CheckSummaryView } from "../bridge/generated/CheckSummaryView";
import { useAccountMutations, useAccountsList } from "../bridge/queries";
import { Badge, Button, Card, DataTable, EmptyState } from "../ui";
import { amount, longDate } from "./format";

const muted = { fontSize: 12, color: "var(--text-muted)" } as const;

export const TREATMENT_LABEL: Record<string, string> = { taxable: "Taxable", isa: "ISA", pension: "Pension" };

export function CheckText({ check }: { check: CheckSummaryView | null }) {
  if (!check) return <span style={muted}>No balance checks</span>;
  return check.passed ? (
    <span style={{ fontSize: 13 }}>Matched on {longDate(check.as_of)}</span>
  ) : (
    <span style={{ fontSize: 13, color: "var(--warn-strong)" }}>
      Mismatch on {longDate(check.as_of)}: {check.failed.join(", ")}
    </span>
  );
}

function MergedRow({ row }: { row: AccountListRow }) {
  const { undoMerge } = useAccountMutations();
  const [skipped, setSkipped] = useState<string[] | null>(null);
  const m = row.merge!;
  return (
    <tr data-testid="merged-row">
      <td style={{ fontWeight: 500 }}>{row.name}</td>
      <td>
        into <Link to={`/accounts/${m.into_id}`}>{m.into_name}</Link>
        <div style={muted}>{longDate(m.merged_at.slice(0, 10))}</div>
      </td>
      <td style={{ textAlign: "right" }}>
        <Button small disabled={undoMerge.isPending} onClick={() => undoMerge.mutate(m.merge_id, { onSuccess: setSkipped })}>
          Undo merge
        </Button>
        {undoMerge.error && (
          <div role="alert" style={{ fontSize: 12, color: "var(--loss)" }}>
            {String(undoMerge.error)}
          </div>
        )}
        {skipped && skipped.length > 0 && (
          <div role="note" style={muted}>
            Left in place, moved since the merge: {skipped.join(", ")}
          </div>
        )}
      </td>
    </tr>
  );
}

export function Accounts() {
  const { data } = useAccountsList();
  if (!data) return <PageHeader title="Accounts" />;
  if (data.accounts.length === 0 && data.merged.length === 0) {
    return (
      <>
        <PageHeader title="Accounts" />
        <EmptyState title="No accounts yet" message="Accounts are created when you import a statement or add an activity." action={{ to: "/imports", label: "Go to Imports" }} />
      </>
    );
  }
  return (
    <>
      <PageHeader title="Accounts" subtitle={`${data.accounts.length} account${data.accounts.length === 1 ? "" : "s"} · values in ${data.base_currency}`} />
      <Card title="Accounts" label="Accounts">
        <DataTable label="Accounts" minWidth={720}>
          <thead>
            <tr>
              <th scope="col">Account</th>
              <th scope="col" style={{ textAlign: "right" }}>
                Value
              </th>
              <th scope="col" style={{ textAlign: "right" }}>
                Activities
              </th>
              <th scope="col">Statement identifiers</th>
              <th scope="col">Tax</th>
              <th scope="col">Latest balance check</th>
            </tr>
          </thead>
          <tbody>
            {data.accounts.map((a) => (
              <tr key={a.id} data-testid="account-list-row">
                <td>
                  <Link to={`/accounts/${a.id}`} style={{ fontWeight: 500 }}>
                    {a.name}
                  </Link>
                </td>
                <td className="num" style={{ textAlign: "right", color: a.value?.startsWith("-") ? "var(--loss)" : undefined }}>
                  {a.value === null ? "–" : amount(a.value)}
                </td>
                <td className="num" style={{ textAlign: "right" }}>
                  {a.activities}
                </td>
                <td style={{ fontSize: 13 }}>{a.identifiers.length ? a.identifiers.join(", ") : <span style={muted}>None</span>}</td>
                <td>
                  <Badge>{TREATMENT_LABEL[a.tax_treatment] ?? a.tax_treatment}</Badge>
                </td>
                <td>
                  <CheckText check={a.latest_check} />
                </td>
              </tr>
            ))}
          </tbody>
        </DataTable>
      </Card>
      {data.merged.length > 0 && (
        <Card title="Merged accounts" label="Merged accounts" aside={<span style={muted}>Undo moves back what the merge moved; later imports stay</span>}>
          <DataTable label="Merged accounts" minWidth={520}>
            <thead>
              <tr>
                <th scope="col">Account</th>
                <th scope="col">Merged</th>
                <th scope="col" />
              </tr>
            </thead>
            <tbody>
              {data.merged.map((m) => (
                <MergedRow key={m.id} row={m} />
              ))}
            </tbody>
          </DataTable>
        </Card>
      )}
    </>
  );
}
