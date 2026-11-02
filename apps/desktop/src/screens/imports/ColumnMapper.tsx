import { keepPreviousData, useMutation, useQuery } from "@tanstack/react-query";
import { save } from "@tauri-apps/plugin-dialog";
import { useMemo, useState } from "react";
import { api, errorMessage } from "../../bridge/api";
import type { MapperInspect } from "../../bridge/generated/MapperInspect";
import type { MapperOptions } from "../../bridge/generated/MapperOptions";
import type { SavedMapping } from "../../bridge/generated/SavedMapping";
import { Badge, Button, Card, DataTable } from "../../ui";
import s from "./imports.module.css";
import { FIELDS, initialDraft, KINDS, toMappingDoc, type FieldChoice, type MappingDraft, type RuleDraft } from "./mapping";

const DELIMITERS: [string, string][] = [[";", "Semicolon ;"], [",", "Comma ,"], ["\t", "Tab"], ["|", "Pipe |"]];
const ENCODINGS = ["utf-8", "windows-1254", "windows-1252", "iso-8859-9"];
const DATE_FORMATS: [string, string][] = [
  ["%d.%m.%Y", "31.12.2026"],
  ["%d/%m/%Y", "31/12/2026"],
  ["%d-%m-%Y", "31-12-2026"],
  ["%Y-%m-%d", "2026-12-31"],
  ["%m/%d/%Y", "12/31/2026"],
  ["%Y%m%d", "20261231"],
];

function Select(props: { label: string; value: string; onChange: (v: string) => void; options: [string, string][]; id: string }) {
  return (
    <label className={s.field} htmlFor={props.id}>
      <span className={s.fieldLabel}>{props.label}</span>
      <select id={props.id} className={s.select} value={props.value} onChange={(e) => props.onChange(e.target.value)}>
        {props.options.map(([v, l]) => (
          <option key={v} value={v}>
            {l}
          </option>
        ))}
      </select>
    </label>
  );
}

function FileOptions({ inspect, draft, setOptions, update }: {
  inspect: MapperInspect;
  draft: MappingDraft;
  setOptions: (o: MapperOptions) => void;
  update: (d: Partial<MappingDraft>) => void;
}) {
  const o = inspect.options;
  const csv = inspect.file_kind === "csv";
  return (
    <Card title="Reading the file">
      <div className={s.sourceGrid}>
        {csv && (
          <Select id="m-delim" label="Delimiter" value={o.delimiter ?? ","} options={DELIMITERS} onChange={(v) => setOptions({ ...o, delimiter: v })} />
        )}
        {csv && (
          <Select id="m-enc" label="Encoding" value={o.encoding ?? "utf-8"} options={ENCODINGS.map((e) => [e, e])} onChange={(v) => setOptions({ ...o, encoding: v === "utf-8" ? null : v })} />
        )}
        {!csv && inspect.sheets.length > 0 && (
          <Select id="m-sheet" label="Sheet" value={o.sheet ?? inspect.sheets[0]!} options={inspect.sheets.map((n) => [n, n])} onChange={(v) => setOptions({ ...o, sheet: v })} />
        )}
        <label className={s.field} htmlFor="m-skip">
          <span className={s.fieldLabel}>Rows before the header</span>
          <input id="m-skip" className={s.select} type="number" min={0} value={o.skip_rows} onChange={(e) => setOptions({ ...o, skip_rows: Math.max(0, Number(e.target.value) || 0) })} />
        </label>
        <Select id="m-dec" label="Decimal separator" value={draft.decimal} options={[[",", "Comma 1,5"], [".", "Point 1.5"]]} onChange={(v) => update({ decimal: v })} />
        <Select
          id="m-thou"
          label="Thousands separator"
          value={draft.thousands}
          options={[[".", "Point 1.000"], [",", "Comma 1,000"], [" ", "Space 1 000"], ["", "None 1000"]]}
          onChange={(v) => update({ thousands: v })}
        />
        <Select
          id="m-date-format"
          label="Date format"
          value={draft.dateFormat}
          options={[...DATE_FORMATS.map(([f, e]): [string, string] => [f, e]), ...(DATE_FORMATS.some(([f]) => f === draft.dateFormat) ? [] : [[draft.dateFormat, draft.dateFormat] as [string, string]])]}
          onChange={(v) => update({ dateFormat: v })}
        />
      </div>
    </Card>
  );
}

function Sample({ inspect }: { inspect: MapperInspect }) {
  return (
    <Card title="First rows" aside={<span className={s.note}>{inspect.total_rows} rows in the file</span>}>
      <DataTable label="First rows of the file" minWidth={640}>
        <thead>
          <tr>
            <th scope="col">Row</th>
            {inspect.header.map((h, i) => (
              <th key={i} scope="col">
                {h || `(column ${i + 1})`}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {inspect.rows.slice(0, 8).map((r) => (
            <tr key={r.row}>
              <td className="num">{r.row}</td>
              {inspect.header.map((_, i) => (
                <td key={i}>{r.cells[i] ?? ""}</td>
              ))}
            </tr>
          ))}
        </tbody>
      </DataTable>
    </Card>
  );
}

function SourcePicker({ id, label, value, header, onChange }: { id: string; label: string; value: FieldChoice; header: string[]; onChange: (c: FieldChoice) => void }) {
  const selected = value.mode === "column" ? `col:${value.column}` : value.mode;
  return (
    <div className={s.mapRow}>
      <label htmlFor={id} className={s.mapLabel}>
        {label}
      </label>
      <select
        id={id}
        className={s.select}
        value={selected}
        onChange={(e) => {
          const v = e.target.value;
          if (v === "none") onChange({ mode: "none", column: "", value: "" });
          else if (v === "const") onChange({ mode: "const", column: "", value: value.value });
          else onChange({ mode: "column", column: v.slice(4), value: "" });
        }}
      >
        <option value="none">Not in the file</option>
        {header.filter(Boolean).map((h) => (
          <option key={h} value={`col:${h}`}>
            Column: {h}
          </option>
        ))}
        <option value="const">Fixed value…</option>
      </select>
      {value.mode === "const" && (
        <input aria-label={`${label} value`} className={s.select} value={value.value} onChange={(e) => onChange({ ...value, value: e.target.value })} />
      )}
    </div>
  );
}

function Fields({ inspect, draft, update }: { inspect: MapperInspect; draft: MappingDraft; update: (d: Partial<MappingDraft>) => void }) {
  return (
    <Card title="Fields">
      <SourcePicker id="m-account" label="Account" value={draft.account} header={inspect.header} onChange={(account) => update({ account })} />
      {FIELDS.map((f) => (
        <SourcePicker
          key={f.key}
          id={`m-field-${f.key}`}
          label={f.help ? `${f.label} (${f.help})` : f.label}
          value={draft.fields[f.key]}
          header={inspect.header}
          onChange={(c) => update({ fields: { ...draft.fields, [f.key]: c } })}
        />
      ))}
      <label className={s.checkLine}>
        <input type="checkbox" checked={draft.absolute} onChange={(e) => update({ absolute: e.target.checked })} />
        Store amounts and quantities without their sign (the kind gives the direction)
      </label>
    </Card>
  );
}

function Rules({ inspect, draft, update }: { inspect: MapperInspect; draft: MappingDraft; update: (d: Partial<MappingDraft>) => void }) {
  const set = (rules: RuleDraft[]) => update({ rules });
  const edit = (i: number, r: Partial<RuleDraft>) => set(draft.rules.map((x, j) => (j === i ? { ...x, ...r } : x)));
  const move = (i: number, d: number) => {
    const next = [...draft.rules];
    const [r] = next.splice(i, 1);
    next.splice(i + d, 0, r!);
    set(next);
  };
  const first = inspect.header.find(Boolean) ?? "";
  return (
    <Card title="Kind rules" aside={<span className={s.note}>The first matching rule wins; rows matching none are not imported.</span>}>
      {draft.rules.length === 0 && <p className={s.note}>Add a rule to decide each row's kind.</p>}
      {draft.rules.map((r, i) => (
        <div key={i} className={s.ruleRow} data-testid="rule">
          <span className={s.small}>{i + 1}.</span>
          <select aria-label={`Rule ${i + 1} column`} className={s.select} value={r.column} onChange={(e) => edit(i, { column: e.target.value })}>
            {inspect.header.filter(Boolean).map((h) => (
              <option key={h} value={h}>
                {h}
              </option>
            ))}
          </select>
          <select aria-label={`Rule ${i + 1} test`} className={s.select} value={r.op} onChange={(e) => edit(i, { op: e.target.value as RuleDraft["op"] })}>
            <option value="equals">is</option>
            <option value="matches">matches pattern</option>
          </select>
          <input aria-label={`Rule ${i + 1} value`} className={s.select} value={r.value} onChange={(e) => edit(i, { value: e.target.value })} />
          <span className={s.small}>→</span>
          <select aria-label={`Rule ${i + 1} kind`} className={s.select} value={r.kind} onChange={(e) => edit(i, { kind: e.target.value })}>
            {[...KINDS, "skip"].map((k) => (
              <option key={k} value={k}>
                {k === "skip" ? "skip the row" : k}
              </option>
            ))}
          </select>
          <span className={s.actions}>
            <Button small variant="ghost" aria-label={`Move rule ${i + 1} up`} disabled={i === 0} onClick={() => move(i, -1)}>
              ↑
            </Button>
            <Button small variant="ghost" aria-label={`Move rule ${i + 1} down`} disabled={i === draft.rules.length - 1} onClick={() => move(i, 1)}>
              ↓
            </Button>
            <Button small variant="ghost" aria-label={`Remove rule ${i + 1}`} onClick={() => set(draft.rules.filter((_, j) => j !== i))}>
              ✕
            </Button>
          </span>
        </div>
      ))}
      <Button small onClick={() => set([...draft.rules, { column: first, op: "equals", value: "", kind: "deposit" }])}>
        Add rule
      </Button>
    </Card>
  );
}

function Preview({ path, doc }: { path: string; doc: object }) {
  const key = JSON.stringify(doc);
  const preview = useQuery({ queryKey: ["mapper-preview", path, key], queryFn: () => api.mapperPreview(path, doc), placeholderData: keepPreviousData });
  const p = preview.data;
  return (
    <section aria-label="Preview">
      <Card title="Preview" aside={p && !p.error ? <span className={s.note}>{p.records.length < p.total_records ? `First ${p.records.length} of ${p.total_records} records` : `${p.total_records} records`}</span> : undefined}>
        {preview.error && <div className={s.error}>{errorMessage(preview.error)}</div>}
        {p?.error && <p className={s.noteWarn}>{p.error}</p>}
        {p && !p.error && p.records.length > 0 && (
          <DataTable label="Preview records" minWidth={720}>
            <thead>
              <tr>
                <th scope="col">Date</th>
                <th scope="col">Kind</th>
                <th scope="col">Account</th>
                <th scope="col">Instrument</th>
                <th scope="col" style={{ textAlign: "right" }}>Qty</th>
                <th scope="col" style={{ textAlign: "right" }}>Amount</th>
                <th scope="col">Source</th>
              </tr>
            </thead>
            <tbody>
              {p.records.map((r) => (
                <tr key={r.locator + r.kind} data-testid="preview-row">
                  <td className="num">{r.date}</td>
                  <td>{r.kind}</td>
                  <td>{r.account}</td>
                  <td>{r.instrument ?? "—"}</td>
                  <td className="num" style={{ textAlign: "right" }}>{r.quantity ?? ""}</td>
                  <td className="num" style={{ textAlign: "right" }}>{r.amount ? `${r.amount} ${r.currency ?? ""}` : ""}</td>
                  <td style={{ color: "var(--text-muted)" }}>{r.locator}</td>
                </tr>
              ))}
            </tbody>
          </DataTable>
        )}
        {p && !p.error && p.diagnostics.length > 0 && (
          <ul className={s.list} aria-label="Rows not imported">
            {p.diagnostics.map((d, i) => (
              <li key={i} data-testid="preview-diagnostic">
                <Badge tone={d.severity === "info" ? "info" : "warn"}>{d.locator}</Badge> {d.message}
              </li>
            ))}
          </ul>
        )}
      </Card>
    </section>
  );
}

function SaveBar({ doc, onClose, onOpenReview }: { doc: object; onClose: () => void; onOpenReview: () => void }) {
  const [saved, setSaved] = useState<SavedMapping | null>(null);
  const saveMapping = useMutation({ mutationFn: api.mapperSave, onSuccess: setSaved });
  const exportMapping = useMutation({
    mutationFn: async (m: SavedMapping) => {
      const dest = await save({ defaultPath: `${m.id}.toml`, filters: [{ name: "Mizan mapping", extensions: ["toml"] }] });
      if (typeof dest === "string") await api.mapperExport(m.id, dest);
    },
  });
  const error = saveMapping.error ?? exportMapping.error;
  return (
    <section aria-label="Save mapping" className={s.commitBar}>
      <div>
        {saved ? (
          <div style={{ fontWeight: 500 }}>Saved “{saved.name}”. Files with these columns will be recognized from now on.</div>
        ) : (
          <div style={{ fontWeight: 500 }}>Save this mapping to reuse it for the next statement</div>
        )}
        {error && <div className={s.commitDetail}>{errorMessage(error)}</div>}
      </div>
      <div className={s.actions}>
        <button type="button" className={s.discard} onClick={onClose}>
          {saved ? "Close" : "Cancel"}
        </button>
        {saved ? (
          <>
            <button type="button" className={s.discard} disabled={exportMapping.isPending} onClick={() => exportMapping.mutate(saved)}>
              Export…
            </button>
            <button type="button" className={s.commitButton} onClick={onOpenReview}>
              Review this file
            </button>
          </>
        ) : (
          <button type="button" className={s.commitButton} disabled={saveMapping.isPending} onClick={() => saveMapping.mutate(doc)}>
            Save mapping
          </button>
        )}
      </div>
    </section>
  );
}

/** Map an unrecognized CSV or XLSX file's columns and save the result as a user mapping. */
export function ColumnMapper({ path, onClose, onOpenReview }: { path: string; onClose: () => void; onOpenReview: () => void }) {
  const [options, setOptions] = useState<MapperOptions | null>(null);
  const inspect = useQuery({ queryKey: ["mapper-inspect", path, options], queryFn: () => api.mapperInspect(path, options), placeholderData: keepPreviousData });
  // Until the user edits anything, the draft is the guess from the file.
  const [edited, setEdited] = useState<MappingDraft | null>(null);
  const data = inspect.data;
  const guessed = useMemo(() => (data ? initialDraft(data) : null), [data]);
  const draft = edited ?? guessed;
  const update = (d: Partial<MappingDraft>) => setEdited((cur) => {
    const base = cur ?? guessed;
    return base ? { ...base, ...d } : cur;
  });
  const doc = useMemo(() => (draft && data ? toMappingDoc({ ...draft, options: data.options }, data) : null), [draft, data]);

  if (inspect.error) return <div role="alert" className={s.error}>{errorMessage(inspect.error)}</div>;
  if (!data || !draft || !doc) return <p className={s.note}>Reading the file…</p>;
  return (
    <>
      <FileOptions inspect={data} draft={draft} setOptions={setOptions} update={update} />
      <Sample inspect={data} />
      <Card title="Name">
        <label className={s.mapRow} htmlFor="m-name">
          <span className={s.mapLabel}>Mapping name</span>
          <input id="m-name" className={s.select} placeholder="My bank CSV" value={draft.name} onChange={(e) => update({ name: e.target.value })} />
        </label>
      </Card>
      <Fields inspect={data} draft={draft} update={update} />
      <Rules inspect={data} draft={draft} update={update} />
      <Preview path={path} doc={doc} />
      <SaveBar doc={doc} onClose={onClose} onOpenReview={onOpenReview} />
    </>
  );
}
