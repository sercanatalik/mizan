import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useMemo, useRef, useState } from "react";
import { useLocation } from "react-router";
import { PageHeader } from "../app/Layout";
import type { PickRequest } from "../app/useMenuEvents";
import { errorMessage } from "../bridge/api";
import type { AssertionCheck } from "../bridge/generated/AssertionCheck";
import type { ImportView } from "../bridge/generated/ImportView";
import type { ReviewItem } from "../bridge/generated/ReviewItem";
import {
  useAccounts,
  useImportClose,
  useImportCommit,
  useImportDecide,
  useImportHistory,
  useImportOpen,
  useImportSession,
  useImportUndo,
  useResolveHint,
} from "../bridge/queries";
import { Badge, Button, Card, DataTable, EmptyState, Segmented, type Tone } from "../ui";
import { buildRows, counts, matches, type Filter, type RecordRow, type Row } from "./imports/model";
import s from "./imports/imports.module.css";

const short = (sha: string | null | undefined) => (sha ? `${sha.slice(0, 4)}…${sha.slice(-4)}` : "—");

// ------------------------------------------------------------------ picker

function usePicker() {
  const openBatch = useImportOpen();
  const [error, setError] = useState<string | null>(null);
  const pick = async () => {
    setError(null);
    try {
      const path = await open({ multiple: false, directory: false, filters: [{ name: "Mizan batch", extensions: ["json"] }] });
      if (typeof path !== "string") return;
      await openBatch.mutateAsync(path);
    } catch (e) {
      setError(errorMessage(e));
    }
  };
  return { pick, error, busy: openBatch.isPending };
}

// ------------------------------------------------------------------ review parts

function SourceCard({ view }: { view: ImportView }) {
  const src = view.source;
  const period = src.first_date ? (src.first_date === src.last_date ? src.first_date : `${src.first_date} – ${src.last_date}`) : "—";
  return (
    <Card label="Batch source">
      <div className={s.sourceGrid}>
        <div className={s.field}>
          <span className={s.fieldLabel}>Adapter</span>
          <span className={s.fieldValue}>{src.adapters.join(", ") || "—"}</span>
        </div>
        <div className={s.field}>
          <span className={s.fieldLabel}>Account</span>
          <span className={s.fieldValue}>{src.accounts.join(", ") || "—"}</span>
        </div>
        <div className={s.field}>
          <span className={s.fieldLabel}>Period</span>
          <span className={s.fieldValue}>{period}</span>
        </div>
        <div className={s.field}>
          <span className={s.fieldLabel}>File</span>
          <span className={`${s.fieldValue} num`}>sha256 {short(src.file_sha256)}</span>
        </div>
      </div>
    </Card>
  );
}

function Stats({ view }: { view: ImportView }) {
  const m = view.report.summary;
  const stats: [string, number, string][] = [
    ["New", m.new, "var(--accent)"],
    ["Duplicates", m.duplicates, "#aeb5ac"],
    ["Need your decision", m.pending_confirmations, "var(--warn)"],
    ["Figure warnings", m.consistency_warnings, "var(--warn)"],
    ["Not imported", m.adapter_diagnostics, "var(--info)"],
    ["Invalid", m.validation_errors, "var(--loss)"],
  ];
  return (
    <section aria-label="Summary" className={s.stats}>
      {stats.map(([label, value, color]) => (
        <div key={label} className={s.stat} data-testid={`stat-${label}`}>
          <span className={s.statLabel}>
            <span className={s.statDot} style={{ background: color }} aria-hidden="true" />
            {label}
          </span>
          <span className={s.statValue}>{value}</span>
        </div>
      ))}
    </section>
  );
}

function CheckIcon({ passed }: { passed: boolean }) {
  return (
    <span className={`${s.checkIcon} ${passed ? s.pass : s.fail}`}>
      <svg role="img" aria-label={passed ? "Passed" : "Failed"} width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round">
        <path d={passed ? "M20 6L9 17l-5-5" : "M18 6L6 18M6 6l12 12"} />
      </svg>
    </span>
  );
}

function BalanceChecks({ checks }: { checks: AssertionCheck[] }) {
  if (checks.length === 0) return null;
  // Failed checks first: they are the most important warning in a review.
  const sorted = [...checks].sort((a, b) => Number(a.passed) - Number(b.passed));
  const failed = checks.filter((c) => !c.passed).length;
  return (
    <section aria-label="Balance checks" className={failed ? s.checkFailed : undefined} style={{ borderRadius: "var(--radius-lg)" }}>
      <Card
        title="Balance checks"
        aside={<span className={failed ? s.failText : s.okText}>{failed ? `${failed} of ${checks.length} failed` : `All ${checks.length} match`}</span>}
      >
        {sorted.map((c, i) => (
          <div key={i}>
            <div className={s.checkHead}>
              <span>
                <strong>{c.account}</strong> · statement closing balances on {c.as_of}
              </span>
              {c.note && <span className={s.noteWarn}>{c.note}</span>}
            </div>
            {c.lines.map((l) => (
              <div key={l.label} className={s.checkRow} data-testid="check-line">
                <CheckIcon passed={l.passed} />
                <span style={{ fontWeight: 500 }}>{l.label}</span>
                <span>
                  <span className={s.small}>Statement</span>
                  <br />
                  <span className="num">{l.expected}</span>
                </span>
                <span>
                  <span className={s.small}>Mizan</span>
                  <br />
                  <span className="num" style={{ color: l.passed ? undefined : "var(--warn-strong)" }}>
                    {l.computed}
                  </span>
                </span>
                <span className={l.passed ? s.okText : s.failText}>{l.passed ? "match" : "differs"}</span>
              </div>
            ))}
          </div>
        ))}
      </Card>
    </section>
  );
}

const STATUS: Record<RecordRow["status"], [string, Tone]> = {
  new: ["New", "accent"],
  duplicate: ["Duplicate", "neutral"],
  confirm: ["Your call", "warn"],
  decided_new: ["New · your call", "accent"],
  decided_duplicate: ["Duplicate · your call", "neutral"],
};

function RecordsTable({ rows, filter }: { rows: Row[]; filter: Filter }) {
  const decide = useImportDecide();
  const visible = rows.filter((r) => matches(r, filter));
  if (visible.length === 0) return <p className={s.note}>Nothing in this view.</p>;
  return (
    <DataTable label="Import records" minWidth={960}>
      <thead>
        <tr>
          <th scope="col">Status</th>
          <th scope="col">Date</th>
          <th scope="col">Kind</th>
          <th scope="col">Instrument</th>
          <th scope="col" style={{ textAlign: "right" }}>Qty</th>
          <th scope="col" style={{ textAlign: "right" }}>Net cash</th>
          <th scope="col">Source</th>
          <th scope="col" style={{ textAlign: "right" }}>
            <span className="visually-hidden">Actions</span>
          </th>
        </tr>
      </thead>
      <tbody>
        {visible.map((row) =>
          row.kind === "diagnostic" ? (
            <tr key={`d-${row.diagnostic.locator}-${row.diagnostic.message}`} data-testid="record-row">
              <td>
                <Badge tone="info">Not imported</Badge>
              </td>
              <td colSpan={5}>
                <div style={{ fontWeight: 500 }}>{row.diagnostic.message}</div>
                <div className={s.note}>Adapter diagnostic ({row.diagnostic.severity})</div>
              </td>
              <td>{row.diagnostic.locator}</td>
              <td />
            </tr>
          ) : (
            <tr key={row.record.index} data-testid="record-row">
              <td>
                <Badge tone={STATUS[row.status][1]}>{STATUS[row.status][0]}</Badge>
              </td>
              <td className="num" style={{ whiteSpace: "nowrap" }}>
                {row.record.date}
              </td>
              <td>{row.record.kind}</td>
              <td>
                <div style={{ fontWeight: 500 }} className="num">
                  {row.record.instrument ?? "—"}
                </div>
                {row.confirmation && (
                  <div className={row.status === "confirm" ? s.noteWarn : s.note}>
                    Same as {row.confirmation.existingLocator} in file {short(row.confirmation.existingFile)}. Second trade or the same one?
                  </div>
                )}
                {row.warnings.map((w) => (
                  <div key={w} className={s.noteWarn}>
                    {w}
                  </div>
                ))}
              </td>
              <td className="num" style={{ textAlign: "right" }}>
                {row.record.quantity ?? ""}
              </td>
              <td className="num" style={{ textAlign: "right", whiteSpace: "nowrap" }}>
                {row.record.net_cash ?? ""}
              </td>
              <td style={{ whiteSpace: "nowrap", color: "var(--text-muted)" }}>{row.record.locator}</td>
              <td style={{ textAlign: "right" }}>
                {row.confirmation && (
                  <span className={s.actions}>
                    {row.status === "confirm" ? (
                      <>
                        <Button small variant="primary" disabled={decide.isPending} onClick={() => decide.mutate({ index: row.record.index, decision: "new" })}>
                          Second trade
                        </Button>
                        <Button small disabled={decide.isPending} onClick={() => decide.mutate({ index: row.record.index, decision: "duplicate" })}>
                          Duplicate
                        </Button>
                      </>
                    ) : (
                      <Button small variant="ghost" disabled={decide.isPending} onClick={() => decide.mutate({ index: row.record.index, decision: null })}>
                        Undo choice
                      </Button>
                    )}
                  </span>
                )}
              </td>
            </tr>
          ),
        )}
      </tbody>
    </DataTable>
  );
}

function Records({ view }: { view: ImportView }) {
  const rows = useMemo(() => buildRows(view.report), [view.report]);
  const n = counts(rows);
  const [filter, setFilter] = useState<Filter>(n.attention > 0 ? "attention" : "all");
  return (
    <Card
      title="Records"
      aside={
        <Segmented
          label="Filter records"
          value={filter}
          onChange={setFilter}
          options={[
            { value: "attention", label: "Needs attention", count: n.attention },
            { value: "new", label: "New", count: n.new },
            { value: "duplicates", label: "Duplicates", count: n.duplicates },
            { value: "all", label: "All", count: n.all },
          ]}
        />
      }
    >
      <RecordsTable rows={rows} filter={filter} />
    </Card>
  );
}

function UnresolvedRow({ item }: { item: ReviewItem }) {
  const { createAccount, mapAccount, createAsset } = useResolveHint();
  const accounts = useAccounts().data ?? [];
  const [target, setTarget] = useState<string>("");
  const busy = createAccount.isPending || mapAccount.isPending || createAsset.isPending;
  if (item.type === "unresolved_account") {
    const hint = item.hint.hint;
    return (
      <div className={s.unresolvedRow}>
        <span>
          Account <strong className="num">{hint}</strong>
          {item.hint.name_hint ? ` (${item.hint.name_hint})` : ""} is not mapped to an account.
        </span>
        <span className={s.actions}>
          <Button small variant="primary" disabled={busy} onClick={() => createAccount.mutate(hint)}>
            Create account
          </Button>
          {accounts.length > 0 && (
            <span className={s.mapForm}>
              <label className="visually-hidden" htmlFor={`map-${hint}`}>
                Existing account for {hint}
              </label>
              <select id={`map-${hint}`} className={s.select} value={target} onChange={(e) => setTarget(e.target.value)}>
                <option value="">Map to existing…</option>
                {accounts.map((a) => (
                  <option key={a.id} value={a.id}>
                    {a.name}
                  </option>
                ))}
              </select>
              <Button small disabled={busy || !target} onClick={() => mapAccount.mutate({ hint, accountId: Number(target) })}>
                Map
              </Button>
            </span>
          )}
        </span>
      </div>
    );
  }
  const key = item.key;
  const label = item.hint.name ?? item.hint.symbol ?? key ?? "instrument";
  return (
    <div className={s.unresolvedRow}>
      <span>
        Instrument <strong>{label}</strong> {key && <span className="num">({key})</span>} is not a known asset.
      </span>
      {key && (
        <Button small variant="primary" disabled={busy} onClick={() => createAsset.mutate(key)}>
          Create asset
        </Button>
      )}
    </div>
  );
}

function OtherSections({ view }: { view: ImportView }) {
  const r = view.report;
  const lists: [string, string[]][] = [
    ["Ordering questions", r.ordering_diagnostics.map((d) => d.message)],
    ["Invalid items", r.validation_errors],
    ["Adjustments (flagged)", r.flagged_adjustments.map((a) => `${a.locator}: ${a.memo ?? "adjustment"}`)],
    ["Unknown cost basis", r.basis_unknown.map((b) => `${b.locator}: ${b.quantity ?? ""} ${b.instrument ?? ""}`.trim())],
  ];
  return (
    <>
      {lists
        .filter(([, items]) => items.length > 0)
        .map(([title, items]) => (
          <Card key={title} title={title}>
            <ul className={s.list}>
              {items.map((t) => (
                <li key={t}>{t}</li>
              ))}
            </ul>
          </Card>
        ))}
    </>
  );
}

function CommitBar({ view }: { view: ImportView }) {
  const commit = useImportCommit();
  const close = useImportClose();
  const m = view.report.summary;
  const failed = view.report.assertion_results.filter((a) => !a.passed).length;
  const blocked = m.unresolved > 0;
  const notes = [
    blocked ? "Resolve the accounts and instruments above before committing" : null,
    m.pending_confirmations ? `${m.pending_confirmations} undecided record${m.pending_confirmations > 1 ? "s" : ""} will be skipped` : null,
    failed ? `${failed} balance check${failed > 1 ? "s" : ""} still failing` : null,
    "this import can be undone as a whole",
  ].filter(Boolean);
  return (
    <section aria-label="Commit" className={s.commitBar}>
      <div>
        <div style={{ fontWeight: 500 }}>
          Commit {m.new} new record{m.new === 1 ? "" : "s"}
        </div>
        <div className={s.commitDetail}>{notes.join(" · ")}</div>
        {commit.error && <div className={s.commitDetail}>Commit failed: {errorMessage(commit.error)}</div>}
      </div>
      <div className={s.actions}>
        <button type="button" className={s.discard} onClick={() => close.mutate()}>
          Discard
        </button>
        <button type="button" className={s.commitButton} disabled={blocked || commit.isPending} onClick={() => commit.mutate()}>
          Commit {m.new}
        </button>
      </div>
    </section>
  );
}

function Review({ view }: { view: ImportView }) {
  const unresolved = view.report.unresolved;
  return (
    <>
      <SourceCard view={view} />
      <Stats view={view} />
      {unresolved.length > 0 && (
        <Card title="Unresolved" aside={<Badge tone="warn">{unresolved.length} to resolve</Badge>}>
          {unresolved.map((u, i) => (
            <UnresolvedRow key={i} item={u} />
          ))}
        </Card>
      )}
      <BalanceChecks checks={view.report.assertion_results} />
      <Records view={view} />
      <OtherSections view={view} />
      <CommitBar view={view} />
    </>
  );
}

// ------------------------------------------------------------------ history

function History() {
  const history = useImportHistory().data ?? [];
  const undo = useImportUndo();
  const [confirming, setConfirming] = useState<number | null>(null);
  if (history.length === 0) return null;
  return (
    <Card title="Import history">
      <DataTable label="Import history" minWidth={720}>
        <thead>
          <tr>
            <th scope="col">Date</th>
            <th scope="col">File</th>
            <th scope="col">Adapter</th>
            <th scope="col">Result</th>
            <th scope="col" style={{ textAlign: "right" }}>
              <span className="visually-hidden">Actions</span>
            </th>
          </tr>
        </thead>
        <tbody>
          {history.map((b) => (
            <tr key={b.id} data-testid="history-row">
              <td className="num">{b.committed_at.slice(0, 10)}</td>
              <td className="num">sha256 {short(b.file_sha256)}</td>
              <td>
                {b.adapter} {b.adapter_version}
              </td>
              <td>
                {b.activity_count} added ·{" "}
                {b.all_checks_passed === null ? "no balance checks" : b.all_checks_passed ? "checks pass" : <span className={s.failText}>checks failed</span>}
              </td>
              <td style={{ textAlign: "right" }}>
                {confirming === b.id ? (
                  <span className={s.actions}>
                    <span className={s.note}>Remove {b.activity_count} records?</span>
                    <Button small variant="primary" disabled={undo.isPending} onClick={() => undo.mutate(b.id, { onSettled: () => setConfirming(null) })}>
                      Confirm undo
                    </Button>
                    <Button small onClick={() => setConfirming(null)}>
                      Cancel
                    </Button>
                  </span>
                ) : (
                  <Button small variant="ghost" onClick={() => setConfirming(b.id)}>
                    Undo
                  </Button>
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </DataTable>
    </Card>
  );
}

// ------------------------------------------------------------------ screen

export function Imports() {
  const session = useImportSession().data;
  const { pick, error, busy } = usePicker();
  const location = useLocation();
  const pickToken = (location.state as PickRequest | null)?.pick;
  const handled = useRef<number | undefined>(undefined);

  // The native menu's "Import batch…" navigates here with a pick request.
  useEffect(() => {
    if (pickToken && handled.current !== pickToken) {
      handled.current = pickToken;
      void pick();
    }
  }, [pickToken]); // eslint-disable-line react-hooks/exhaustive-deps

  return (
    <>
      <PageHeader
        title="Imports"
        subtitle={session ? `Review · ${session.file_name}` : "Open a mizan.import/1 batch file to review it before anything is saved."}
        actions={
          <Button variant={session ? "secondary" : "primary"} onClick={() => void pick()} disabled={busy}>
            {session ? "Open another batch" : "Open batch file"}
          </Button>
        }
      />
      {error && (
        <div role="alert" className={s.error}>
          {error}
        </div>
      )}
      {session ? (
        <Review view={session} />
      ) : (
        <EmptyState
          title="No import in review"
          message="Statement files (CSV, PDF) need an adapter. Until adapters are available, open a mizan.import/1 batch file."
        />
      )}
      <History />
    </>
  );
}
