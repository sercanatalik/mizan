import { useEffect, useState, type ReactNode } from "react";
import { Link, useLocation, useSearchParams } from "react-router";
import { PageHeader } from "../app/Layout";
import type { AddActivityRequest } from "../app/ViewBar";
import { api, errorMessage } from "../bridge/api";
import type { ActivityDetail } from "../bridge/generated/ActivityDetail";
import type { ActivityForm } from "../bridge/generated/ActivityForm";
import type { ActivityOptions } from "../bridge/generated/ActivityOptions";
import type { ActivityQuery } from "../bridge/generated/ActivityQuery";
import type { ActivityRow } from "../bridge/generated/ActivityRow";
import type { FormCheck } from "../bridge/generated/FormCheck";
import type { MoneyView } from "../bridge/generated/MoneyView";
import { useActivities, useActivityDetail, useActivityMutations, useActivityOptions } from "../bridge/queries";
import { Badge, Button, Card, DataTable, EmptyState, Segmented } from "../ui";
import s from "./activity.module.css";
import { amount, longDate } from "./format";

const muted = { fontSize: 12, color: "var(--text-muted)" } as const;
const right = { textAlign: "right" } as const;

const KIND_LABELS: Record<string, string> = {
  buy: "Buy",
  sell: "Sell",
  deposit: "Deposit",
  withdrawal: "Withdrawal",
  dividend: "Dividend",
  interest: "Interest",
  fee: "Fee",
  tax: "Tax",
  position_in: "Transfer in",
  position_out: "Transfer out",
  split: "Split",
};

export function kindLabel(kind: string): string {
  return KIND_LABELS[kind] ?? kind.replace(/_/g, " ").replace(/^./, (c) => c.toUpperCase());
}

type Source = "all" | "imported" | "manual";
const SOURCES: { value: Source; label: string }[] = [
  { value: "all", label: "All" },
  { value: "imported", label: "Imported" },
  { value: "manual", label: "Manual" },
];

const EMPTY_QUERY: ActivityQuery = { accounts: [], kinds: [], from: null, to: null, source: null, text: null };

/** Filters from a link: ?account=<id>&text=<instrument>&kinds=a,b&from=<date>&to=<date>. */
function queryFromParams(params: URLSearchParams): ActivityQuery {
  const account = Number(params.get("account"));
  return {
    ...EMPTY_QUERY,
    accounts: account ? [account] : [],
    text: params.get("text") || null,
    kinds: params.get("kinds")?.split(",").filter(Boolean) ?? [],
    from: params.get("from") || null,
    to: params.get("to") || null,
  };
}

function money(m: MoneyView | null): string {
  return m ? `${amount(m.amount)} ${m.currency}` : "—";
}

function sourceText(r: ActivityRow): string {
  return r.source.manual ? "manual" : `${r.source.adapter}, ${r.source.locator}`;
}

/** The "Edited" tag, with when the activity was last changed. */
function EditedTag({ r }: { r: ActivityRow }) {
  if (!r.source.edited) return null;
  const when = r.source.edited_at ? `Edited ${longDate(r.source.edited_at.slice(0, 10))}` : "Edited";
  return (
    <span title={when} aria-label={when} data-testid="edited-tag">
      <Badge tone="info">Edited</Badge>
    </span>
  );
}

// ------------------------------------------------------------------ filters

function Filters({
  query,
  set,
  options,
  onClear,
}: {
  query: ActivityQuery;
  set: (q: ActivityQuery) => void;
  options: ActivityOptions | undefined;
  onClear: (() => void) | null;
}) {
  const source: Source = (query.source as Source | null) ?? "all";
  return (
    <section aria-label="Filters" className={s.toolbar}>
      <select
        aria-label="Account"
        className={`${s.control} ${s.account}`}
        value={query.accounts[0] ?? ""}
        onChange={(e) => set({ ...query, accounts: e.target.value ? [Number(e.target.value)] : [] })}
      >
        <option value="">All accounts</option>
        {options?.accounts.map((a) => (
          <option key={a.id} value={a.id}>
            {a.label}
          </option>
        ))}
      </select>
      <select aria-label="Kind" className={s.control} value={query.kinds[0] ?? ""} onChange={(e) => set({ ...query, kinds: e.target.value ? [e.target.value] : [] })}>
        <option value="">All kinds</option>
        {options?.kinds.map((k) => (
          <option key={k} value={k}>
            {kindLabel(k)}
          </option>
        ))}
      </select>
      <input
        aria-label="Instrument"
        type="search"
        className={`${s.control} ${s.search}`}
        placeholder="Symbol, name or ISIN"
        value={query.text ?? ""}
        onChange={(e) => set({ ...query, text: e.target.value || null })}
      />
      <span className={s.range}>
        <input type="date" aria-label="From" className={`${s.control} ${s.date}`} value={query.from ?? ""} onChange={(e) => set({ ...query, from: e.target.value || null })} />
        –
        <input type="date" aria-label="To" className={`${s.control} ${s.date}`} value={query.to ?? ""} onChange={(e) => set({ ...query, to: e.target.value || null })} />
      </span>
      <Segmented label="Source" value={source} onChange={(v) => set({ ...query, source: v === "all" ? null : v })} options={SOURCES} />
      {onClear && (
        <Button small variant="ghost" onClick={onClear}>
          Clear filters
        </Button>
      )}
    </section>
  );
}

// ------------------------------------------------------------------ detail

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <>
      <dt style={muted}>{label}</dt>
      <dd style={{ margin: 0 }}>{children}</dd>
    </>
  );
}

function Detail({ d, onEdit, onClose }: { d: ActivityDetail; onEdit: () => void; onClose: () => void }) {
  const { remove, revert } = useActivityMutations();
  const [confirming, setConfirming] = useState(false);
  const r = d.row;
  return (
    <Card
      title={
        <span style={{ display: "inline-flex", alignItems: "center", gap: 8 }}>
          {`${kindLabel(r.kind)} · ${longDate(r.date)}`}
          {r.source.edited && (
            <span data-testid="edited-detail">
              <Badge tone="info">{r.source.edited_at ? `Edited ${longDate(r.source.edited_at.slice(0, 10))}` : "Edited"}</Badge>
            </span>
          )}
        </span>
      }
      label="Activity detail" aside={<Button small variant="ghost" onClick={onClose}>Close</Button>}>
      <dl style={{ display: "grid", gridTemplateColumns: "max-content 1fr", gap: "6px 14px", margin: 0, fontSize: 13 }} data-testid="activity-detail">
        <Field label="Account">{r.account}</Field>
        {r.instrument && <Field label="Instrument">{r.instrument}{r.isin && <span style={{ ...muted, display: "block" }}>{r.isin}</span>}</Field>}
        {r.quantity && <Field label="Quantity">{r.quantity}</Field>}
        {r.price && <Field label="Price">{money(r.price)}</Field>}
        {d.gross && <Field label="Gross">{money(d.gross)}</Field>}
        {r.net_cash && <Field label="Net cash">{money(r.net_cash)}</Field>}
        {d.charges.map((c, i) => (
          <Field key={i} label={kindLabel(c.kind)}>{`${amount(c.amount)} ${c.currency}`}</Field>
        ))}
        {d.fx && <Field label="Exchange rate">{d.fx}</Field>}
        {d.split && <Field label="Split">{d.split}</Field>}
        {d.cost_basis && <Field label="Cost basis">{money(d.cost_basis)}</Field>}
        {d.acquisition_date && <Field label="Acquired">{longDate(d.acquisition_date)}</Field>}
        {d.settle_date && <Field label="Settles">{longDate(d.settle_date)}</Field>}
        {d.legs.map((l, i) => (
          <Field key={`leg-${i}`} label="Leg">{`${l.instrument} ${l.quantity_delta}${l.cost_basis_share ? ` · cost share ${l.cost_basis_share}` : ""}`}</Field>
        ))}
        {d.links.map((l) => (
          <Field key={l} label="Link">{l}</Field>
        ))}
        {d.external_id && <Field label="Reference">{d.external_id}</Field>}
        {d.memo && <Field label="Memo">{d.memo}</Field>}
        <Field label="Source">
          {r.source.manual ? (
            <>manual entry{r.source.added_at && <span style={muted}> · added {longDate(r.source.added_at.slice(0, 10))}</span>}</>
          ) : (
            <>
              {r.source.adapter} {r.source.adapter_version} · {r.source.locator}
              <span style={{ ...muted, display: "block" }}>file sha256 {d.file_sha256.slice(0, 12)}</span>
            </>
          )}
        </Field>
      </dl>
      {d.import && (
        <p style={{ ...muted, margin: 0 }} data-testid="imported-note">
          {d.amended_at
            ? `Amended ${longDate(d.amended_at.slice(0, 10))}; the imported values are kept and can be restored.`
            : "Imported. Edits keep the imported values so they can be restored."}{" "}
          To remove it, undo its import:{" "}
          <Link to={`/imports#batch-${d.import.batch_id}`}>Open its import ({d.import.committed_at.slice(0, 10)})</Link>
        </p>
      )}
      <div style={{ display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap" }}>
        {r.editable && <Button onClick={onEdit}>Edit</Button>}
        {r.source.amended && (
          <Button variant="ghost" disabled={revert.isPending} onClick={() => revert.mutate(r.id)}>
            Revert to imported values
          </Button>
        )}
        {r.deletable &&
          (confirming ? (
            <>
              <span style={{ fontSize: 13 }}>Delete this entry?</span>
              <Button variant="primary" disabled={remove.isPending} onClick={() => remove.mutate(r.id, { onSuccess: onClose })}>
                Delete
              </Button>
              <Button variant="ghost" onClick={() => setConfirming(false)}>
                Keep
              </Button>
            </>
          ) : (
            <Button variant="ghost" onClick={() => setConfirming(true)}>
              Delete…
            </Button>
          ))}
        {(remove.error || revert.error) && <span style={{ fontSize: 12, color: "var(--loss)" }}>{errorMessage(remove.error ?? revert.error)}</span>}
      </div>
    </Card>
  );
}

// ------------------------------------------------------------------ form

const NEEDS_ASSET = ["buy", "sell", "dividend", "position_in", "position_out", "split"];
const NO_CURRENCY = ["position_out", "split"];
const NEW = "new";

function blankForm(options: ActivityOptions | undefined, request: AddActivityRequest["add"] | null = null): ActivityForm {
  const accountId = request?.accountId ?? null;
  const preferred = accountId !== null && options?.accounts.some((a) => a.id === accountId) ? accountId : null;
  const asset = request?.assetId != null && options?.assets.some((a) => a.id === request.assetId) ? request.assetId : null;
  return {
    kind: request?.kind ?? "buy",
    date: new Date().toISOString().slice(0, 10),
    account_id: preferred ?? options?.accounts[0]?.id ?? null,
    new_account: null,
    asset_id: asset,
    new_asset: null,
    currency: "",
    quantity: null,
    price: null,
    net_cash: null,
    gross: null,
    commission: null,
    withholding: null,
    cost_basis: null,
    acquisition_date: null,
    split_new: null,
    split_old: null,
    memo: null,
  };
}

function useCheck(form: ActivityForm, replaceId: number | null): FormCheck | null {
  const [check, setCheck] = useState<FormCheck | null>(null);
  useEffect(() => {
    const t = setTimeout(() => {
      api.activityCheck(form, replaceId).then(setCheck, () => setCheck(null));
    }, 300);
    return () => clearTimeout(t);
  }, [form, replaceId]);
  return check;
}

type FieldName = keyof ActivityForm | "account" | "asset";

/** One labelled control with its error, shown once the field was left or Save was pressed. */
function Labelled({ label, hint, error, wide, children }: { label: string; hint?: string; error?: string; wide?: boolean; children: ReactNode }) {
  return (
    <label className={`${s.field} ${wide ? s.wide : ""}`}>
      <span className={s.label}>
        {label}
        {hint && <span className={s.hint}> · {hint}</span>}
      </span>
      {children}
      {error && (
        <span role="alert" className={s.error}>
          {error}
        </span>
      )}
    </label>
  );
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <fieldset className={s.section}>
      <legend className={s.sectionTitle}>{title}</legend>
      <div className={s.grid}>{children}</div>
    </fieldset>
  );
}

function EntryForm({
  options,
  initial,
  replaceId,
  onDone,
  onCancel,
  imported = false,
}: {
  options: ActivityOptions;
  initial: ActivityForm;
  replaceId: number | null;
  onDone: (id: number) => void;
  onCancel: () => void;
  imported?: boolean;
}) {
  const [f, setF] = useState<ActivityForm>(initial);
  const [shown, setShown] = useState<FormCheck | null>(null);
  const [duplicate, setDuplicate] = useState(false);
  const [touched, setTouched] = useState<Set<string>>(new Set());
  const [submitted, setSubmitted] = useState(false);
  const live = useCheck(f, replaceId);
  const { save } = useActivityMutations();
  const check = shown ?? live;
  const set = (patch: Partial<ActivityForm>) => {
    setF({ ...f, ...patch });
    setShown(null);
    setDuplicate(false);
  };
  const touch = (name: string) => () => setTouched((t) => (t.has(name) ? t : new Set(t).add(name)));
  // Errors appear for a field once it was left, or for every field after Save.
  const err = (name: string) => (submitted || touched.has(name) ? check?.errors.find((e) => e.field === name)?.message : undefined);
  const control = (name: FieldName, extra = "") => ({
    className: `${s.control} ${extra}`,
    onBlur: touch(name),
    "aria-invalid": err(name) ? true : undefined,
  });
  const text = (name: keyof ActivityForm, label: string, opts: { hint?: string; type?: string; numeric?: boolean; wide?: boolean } = {}) => (
    <Labelled key={name} label={label} hint={opts.hint} error={err(name)} wide={opts.wide}>
      <input
        aria-label={label}
        type={opts.type ?? "text"}
        inputMode={opts.numeric ? "decimal" : undefined}
        {...control(name, opts.numeric ? s.number : "")}
        value={(f[name] as string | null) ?? ""}
        onChange={(e) => set({ [name]: e.target.value || null } as Partial<ActivityForm>)}
      />
    </Labelled>
  );
  const num = (name: keyof ActivityForm, label: string, hint?: string) => text(name, label, { numeric: true, hint });
  const submit = (confirm: boolean) => {
    setSubmitted(true);
    save.mutate(
      { form: f, confirmDuplicate: confirm, replaceId },
      {
        onSuccess: (out) => {
          if (out.saved_id !== null) onDone(out.saved_id);
          else {
            setShown(out.check);
            setDuplicate(out.needs_confirmation);
          }
        },
      },
    );
  };
  const kind = f.kind;
  const newAsset = f.new_asset;
  // An imported kind the full form does not cover: only its common fields.
  const generic = !options.manual_kinds.includes(kind);
  const formErrors = submitted ? (check?.errors.filter((e) => e.field === "form") ?? []) : [];
  const notices = [...(check?.warnings ?? []), ...formErrors.map((e) => e.message)];
  return (
    <Card title={replaceId ? (imported ? "Amend imported activity" : "Edit activity") : "Add activity"} label="Activity form">
      <form
        className={s.form}
        onSubmit={(e) => {
          e.preventDefault();
          submit(duplicate);
        }}
      >
        <Section title="Activity">
          <Labelled label="Kind">
            <select aria-label="Kind of activity" className={s.control} value={kind} disabled={generic} onChange={(e) => set({ kind: e.target.value })}>
              {(generic ? [kind] : options.manual_kinds).map((k) => (
                <option key={k} value={k}>
                  {kindLabel(k)}
                </option>
              ))}
            </select>
          </Labelled>
          {text("date", "Date", { type: "date" })}
          <Labelled label="Account" error={err("account")}>
            <select
              aria-label="Account of activity"
              {...control("account")}
              value={f.account_id ?? NEW}
              onChange={(e) => set(e.target.value === NEW ? { account_id: null, new_account: "" } : { account_id: Number(e.target.value), new_account: null })}
            >
              {options.accounts.map((a) => (
                <option key={a.id} value={a.id}>
                  {a.label}
                </option>
              ))}
              <option value={NEW}>New account…</option>
            </select>
          </Labelled>
          {f.account_id === null && text("new_account", "New account name")}
        </Section>

        {generic ? (
          <Section title="Currency">
            <Labelled label="Currency" hint="of the amounts below" error={err("currency")}>
              <input aria-label="Currency" maxLength={3} {...control("currency")} value={f.currency} onChange={(e) => set({ currency: e.target.value.toUpperCase() })} />
            </Labelled>
          </Section>
        ) : NEEDS_ASSET.includes(kind) || !NO_CURRENCY.includes(kind) ? (
          <Section title={NEEDS_ASSET.includes(kind) ? "Instrument" : "Currency"}>
            {NEEDS_ASSET.includes(kind) && (
              <Labelled label="Asset" error={err("asset")}>
                <select
                  aria-label="Asset of activity"
                  {...control("asset")}
                  value={newAsset ? NEW : (f.asset_id ?? "")}
                  onChange={(e) => {
                    if (e.target.value === NEW) set({ asset_id: null, new_asset: { symbol: null, name: null, isin: null, currency: null, class: null } });
                    else {
                      const a = options.assets.find((x) => x.id === Number(e.target.value));
                      set({ asset_id: a ? a.id : null, new_asset: null, currency: f.currency || a?.currency || "" });
                    }
                  }}
                >
                  <option value="">Choose an asset…</option>
                  {options.assets.map((a) => (
                    <option key={a.id} value={a.id}>
                      {a.label}
                    </option>
                  ))}
                  <option value={NEW}>New asset…</option>
                </select>
              </Labelled>
            )}
            {!NO_CURRENCY.includes(kind) && (
              <Labelled label="Currency" hint="e.g. USD" error={err("currency")}>
                <input
                  aria-label="Currency"
                  maxLength={3}
                  {...control("currency")}
                  value={f.currency}
                  onChange={(e) => set({ currency: e.target.value.toUpperCase() })}
                />
              </Labelled>
            )}
            {newAsset &&
              (
                [
                  ["symbol", "Symbol"],
                  ["name", "Name"],
                  ["isin", "ISIN"],
                  ["currency", "Quoted in"],
                ] as const
              ).map(([k, label]) => (
                <Labelled key={k} label={label}>
                  <input
                    aria-label={`New asset ${k}`}
                    className={s.control}
                    value={newAsset[k] ?? ""}
                    onChange={(e) => set({ new_asset: { ...newAsset, [k]: e.target.value || null } })}
                  />
                </Labelled>
              ))}
            {newAsset && (
              <Labelled label="Class">
                <select aria-label="New asset class" className={s.control} value={newAsset.class ?? ""} onChange={(e) => set({ new_asset: { ...newAsset, class: e.target.value || null } })}>
                  <option value="">—</option>
                  {["equity", "etf", "fund", "bond", "commodity", "other"].map((c) => (
                    <option key={c} value={c}>
                      {c}
                    </option>
                  ))}
                </select>
              </Labelled>
            )}
          </Section>
        ) : null}

        <Section title="Amounts">
          {(kind === "buy" || kind === "sell") && [num("quantity", "Quantity"), num("price", "Price"), num("commission", "Commission", "optional"), num("net_cash", "Total", "optional, worked out if empty")]}
          {["deposit", "withdrawal", "interest", "fee", "tax"].includes(kind) && num("net_cash", "Amount")}
          {kind === "dividend" && [num("gross", "Gross"), num("withholding", "Tax withheld", "optional"), num("net_cash", "Received", "optional")]}
          {kind === "position_in" && [num("quantity", "Quantity"), num("cost_basis", "Cost basis", "optional"), text("acquisition_date", "Acquired on", { type: "date", hint: "optional" })]}
          {kind === "position_out" && num("quantity", "Quantity")}
          {kind === "split" && [num("split_new", "New units"), num("split_old", "Old units")]}
          {generic && [num("quantity", "Quantity", "optional"), num("price", "Price", "optional"), num("gross", "Gross", "optional"), num("net_cash", "Net cash", "optional")]}
          {text("memo", "Memo", { hint: "optional", wide: true })}
        </Section>

        {(notices.length > 0 || check?.duplicate) && (
          <div className={s.notice} role="status">
            {notices.map((w) => (
              <span key={w} data-testid="form-warning">
                {w}
              </span>
            ))}
            {check?.duplicate && (
              <span data-testid="duplicate-warning">
                This may be a duplicate of the {longDate(check.duplicate.date)} activity from {check.duplicate.source}.
              </span>
            )}
          </div>
        )}

        <div className={s.actions}>
          {save.error && <span className={s.error}>{errorMessage(save.error)}</span>}
          <Button variant="ghost" onClick={onCancel}>
            Cancel
          </Button>
          <Button variant="primary" type="submit" disabled={save.isPending}>
            {duplicate ? "Save anyway" : "Save"}
          </Button>
        </div>
      </form>
    </Card>
  );
}

// ------------------------------------------------------------------ list

function Table({ rows, onOpen }: { rows: ActivityRow[]; onOpen: (id: number) => void }) {
  return (
    <DataTable label="Activities" minWidth={980}>
      <thead>
        <tr>
          <th scope="col">Date</th>
          <th scope="col">Account</th>
          <th scope="col">Kind</th>
          <th scope="col">Instrument</th>
          <th scope="col" style={right}>Quantity</th>
          <th scope="col" style={right}>Price</th>
          <th scope="col" style={right}>Net cash</th>
          <th scope="col" style={right}>Charges</th>
          <th scope="col">Source</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((r) => (
          <tr key={r.id} data-testid="activity-row">
            <td className="num">
              <button
                type="button"
                onClick={() => onOpen(r.id)}
                aria-label={`Open ${kindLabel(r.kind)} on ${longDate(r.date)}`}
                style={{ all: "unset", cursor: "pointer", textDecoration: "underline dotted" }}
              >
                {longDate(r.date)}
              </button>
            </td>
            <td>{r.account}</td>
            <td>
              <span style={{ display: "inline-flex", alignItems: "center", gap: 6 }}>
                {kindLabel(r.kind)}
                <EditedTag r={r} />
              </span>
            </td>
            <td>{r.instrument ?? "—"}</td>
            <td className="num" style={right}>{r.quantity ?? ""}</td>
            <td className="num" style={right}>{r.price ? money(r.price) : ""}</td>
            <td className="num" style={right}>{r.net_cash ? money(r.net_cash) : ""}</td>
            <td className="num" style={right}>{r.charges.map((c) => money(c)).join(" · ")}</td>
            <td style={muted}>{sourceText(r)}</td>
          </tr>
        ))}
      </tbody>
    </DataTable>
  );
}

type Panel = { type: "detail"; id: number } | { type: "add" } | { type: "edit"; id: number; form: ActivityForm; imported: boolean } | null;

export function Activity() {
  // The account page links here as /activity?account=<id>.
  const [params] = useSearchParams();
  const [query, setQuery] = useState<ActivityQuery>(() => queryFromParams(params));
  const [pages, setPages] = useState(1);
  // The Add menu asks for the form through location state, with the scoped account.
  const addRequest = (useLocation().state as AddActivityRequest | null)?.add;
  const [panel, setPanel] = useState<Panel>(addRequest ? { type: "add" } : null);
  const [addFor, setAddFor] = useState<AddActivityRequest["add"] | null>(addRequest ?? null);
  if (addRequest && addRequest.at !== addFor?.at) {
    setAddFor(addRequest);
    setPanel({ type: "add" });
  }
  const { data } = useActivities(query, pages);
  const { data: options } = useActivityOptions();
  const { data: detail } = useActivityDetail(panel?.type === "detail" ? panel.id : null);
  const setQ = (q: ActivityQuery) => {
    setQuery(q);
    setPages(1);
  };
  const filtered = JSON.stringify(query) !== JSON.stringify(EMPTY_QUERY);
  const addButton = (
    <Button variant="primary" onClick={() => setPanel({ type: "add" })} disabled={!options}>
      Add activity
    </Button>
  );
  const side =
    panel?.type === "detail" && detail ? (
      <Detail d={detail} onClose={() => setPanel(null)} onEdit={() => setPanel({ type: "edit", id: detail.row.id, form: detail.form, imported: !detail.row.source.manual })} />
    ) : (panel?.type === "add" || panel?.type === "edit") && options ? (
      <EntryForm
        key={panel.type === "edit" ? `edit-${panel.id}` : "add"}
        options={options}
        initial={panel.type === "edit" ? panel.form : blankForm(options, addFor)}
        replaceId={panel.type === "edit" ? panel.id : null}
        imported={panel.type === "edit" && panel.imported}
        onDone={(id) => setPanel({ type: "detail", id })}
        onCancel={() => setPanel(null)}
      />
    ) : null;

  if (data && !data.has_any) {
    return (
      <>
        <PageHeader title="Activity" actions={addButton} />
        {side}
        {!side && (
          <EmptyState
            title="No activities yet"
            message="Import a statement batch, or add an activity by hand with Add activity."
            action={{ to: "/imports", label: "Go to Imports" }}
          />
        )}
      </>
    );
  }
  return (
    <>
      <PageHeader title="Activity" subtitle={data ? `${data.total} ${data.total === 1 ? "activity" : "activities"}${filtered ? " match" : ""}` : undefined} actions={addButton} />
      <Filters query={query} set={setQ} options={options} onClear={filtered ? () => setQ(EMPTY_QUERY) : null} />
      {side}
      {data && (
        <section aria-label="Totals" data-testid="totals" style={{ display: "flex", flexWrap: "wrap", gap: 16, fontSize: 13 }}>
          <span data-testid="match-count">
            {data.total} {data.total === 1 ? "activity" : "activities"}
          </span>
          {data.totals.map((t) => (
            <span key={t.currency} className="num">
              {t.currency}: in {amount(t.cash_in)} · out {amount(t.cash_out)} · net {amount(t.net)}
            </span>
          ))}
        </section>
      )}
      {data && data.total === 0 ? (
        <Card>
          <p style={{ margin: 0, fontSize: 13 }}>No activities match these filters. Use Clear filters above to see everything again.</p>
        </Card>
      ) : (
        data && (
          <Card label="Activities">
            <Table rows={data.rows} onOpen={(id) => setPanel({ type: "detail", id })} />
            {data.rows.length < data.total && (
              <Button onClick={() => setPages(pages + 1)}>
                Show more ({data.total - data.rows.length} left)
              </Button>
            )}
          </Card>
        )
      )}
    </>
  );
}
