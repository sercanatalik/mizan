// The column mapper's draft and its conversion to a mapping file document
// (the same shape as adapter.toml, sent to Rust as JSON).
import type { MapperInspect } from "../../bridge/generated/MapperInspect";
import type { MapperOptions } from "../../bridge/generated/MapperOptions";

export type FieldKey = "date" | "settle_date" | "amount" | "currency" | "quantity" | "price" | "symbol" | "isin" | "name" | "external_id" | "memo";

export const FIELDS: { key: FieldKey; label: string; help?: string }[] = [
  { key: "date", label: "Date", help: "required" },
  { key: "amount", label: "Amount", help: "net cash moved" },
  { key: "currency", label: "Currency" },
  { key: "quantity", label: "Quantity" },
  { key: "price", label: "Unit price" },
  { key: "symbol", label: "Symbol" },
  { key: "isin", label: "ISIN" },
  { key: "name", label: "Instrument name" },
  { key: "settle_date", label: "Settlement date" },
  { key: "external_id", label: "Transaction id" },
  { key: "memo", label: "Description" },
];

export const KINDS = [
  "deposit", "withdrawal", "buy", "sell", "dividend", "interest", "distribution", "fee", "tax",
  "fx_conversion", "internal_transfer", "short_sell", "buy_to_cover", "position_in", "position_out", "adjustment",
];

export interface FieldChoice {
  mode: "none" | "column" | "const";
  column: string;
  value: string;
}

export interface RuleDraft {
  column: string;
  op: "equals" | "matches";
  value: string;
  /** A record kind, or "skip". */
  kind: string;
}

export interface MappingDraft {
  name: string;
  options: MapperOptions;
  decimal: string;
  thousands: string;
  dateFormat: string;
  account: FieldChoice;
  fields: Record<FieldKey, FieldChoice>;
  /** Amounts and quantities are stored as magnitudes; the kind gives the direction. */
  absolute: boolean;
  rules: RuleDraft[];
}

const none = (): FieldChoice => ({ mode: "none", column: "", value: "" });
const col = (column: string): FieldChoice => ({ mode: "column", column, value: "" });
const constant = (value: string): FieldChoice => ({ mode: "const", column: "", value });

const GUESS: Partial<Record<FieldKey, RegExp>> = {
  date: /^(işlem tarihi|tarih|date|datum|trade date|booking date|valuta)/i,
  amount: /^(tutar|amount|bedrag|betrag|net amount|montant)/i,
  currency: /^(para birimi|currency|valuta|währung|ccy)$/i,
  quantity: /^(adet|quantity|aantal|anzahl|units|qty)/i,
  price: /^(fiyat|price|koers|kurs)/i,
  symbol: /^(symbol|ticker|sembol|kod)/i,
  isin: /^isin/i,
  name: /^(product|produkt|fon adı|instrument|security)/i,
  external_id: /^(dekont|referans|reference|transaction id|order id|id)/i,
  memo: /^(açıklama|description|omschrijving|beschreibung|details|memo)/i,
};

/** "My Bank CSV" → "my-bank-csv" (ASCII, as adapter ids require). */
export function slug(name: string): string {
  const folded = name
    .toLowerCase()
    .replace(/ı/g, "i").replace(/ş/g, "s").replace(/ğ/g, "g").replace(/ü/g, "u").replace(/ö/g, "o").replace(/ç/g, "c")
    .normalize("NFKD")
    .replace(/[̀-ͯ]/g, "");
  return folded.replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 48);
}

/** A starting draft: columns guessed from header names, sign-based deposit/withdrawal rules. */
export function initialDraft(inspect: MapperInspect): MappingDraft {
  const fields = Object.fromEntries(FIELDS.map((f) => [f.key, none()])) as Record<FieldKey, FieldChoice>;
  const used = new Set<string>();
  for (const f of FIELDS) {
    const re = GUESS[f.key];
    // Turkish "İ" only lowercases to "i" with the Turkish locale; English headers need the default one.
    const hit = re && inspect.header.find((h) => !used.has(h) && [h.trim(), h.trim().toLocaleLowerCase("tr")].some((t) => re.test(t)));
    if (hit) {
      fields[f.key] = col(hit);
      used.add(hit);
    }
  }
  if (fields.currency.mode === "none") fields.currency = constant("TRY");
  const amount = fields.amount.mode === "column" ? fields.amount.column : "";
  const stem = inspect.file_name.replace(/\.[^.]+$/, "");
  const turkish = inspect.options.encoding === "windows-1254" || inspect.options.delimiter === ";";
  return {
    name: "",
    options: inspect.options,
    decimal: turkish ? "," : ".",
    thousands: turkish ? "." : ",",
    dateFormat: turkish ? "%d.%m.%Y" : "%Y-%m-%d",
    account: constant(stem),
    fields,
    absolute: true,
    rules: amount
      ? [
          { column: amount, op: "matches", value: "^-", kind: "withdrawal" },
          { column: amount, op: "matches", value: ".", kind: "deposit" },
        ]
      : [],
  };
}

function source(c: FieldChoice, transform: string[] = []): object | undefined {
  if (c.mode === "column" && c.column) return transform.length ? { column: c.column, transform } : { column: c.column };
  if (c.mode === "const" && c.value.trim()) return { const: c.value.trim() };
  return undefined;
}

/** Drop undefined values so the document stays valid TOML. */
function compact<T extends object>(o: T): T {
  return Object.fromEntries(Object.entries(o).filter(([, v]) => v !== undefined && v !== null)) as T;
}

/** The mapping file document for a draft. */
export function toMappingDoc(draft: MappingDraft, inspect: MapperInspect): object {
  const f = draft.fields;
  const abs = draft.absolute ? ["abs"] : [];
  const currency = source(f.currency);
  const amount = source(f.amount, abs);
  const price = source(f.price);
  const instrument = compact({ symbol: source(f.symbol), isin: source(f.isin), name: source(f.name) });
  const fields = compact({
    date: source(f.date),
    settle_date: source(f.settle_date),
    quantity: source(f.quantity, abs),
    net_cash: amount && currency ? { amount, currency } : undefined,
    price: price && currency ? { amount: price, currency } : undefined,
    instrument: Object.keys(instrument).length ? instrument : undefined,
    external_id: f.external_id.mode !== "none" ? { ...source(f.external_id), optional: true } : undefined,
    memo: source(f.memo),
  });
  // Every column the mapping reads must be present for a file to match it.
  const columns = new Set<string>();
  for (const c of [draft.account, ...Object.values(f)]) if (c.mode === "column" && c.column) columns.add(c.column);
  for (const r of draft.rules) if (r.column) columns.add(r.column);
  const name = draft.name.trim() || "My mapping";
  return {
    adapter: compact({ id: slug(name) || "my-mapping", version: "1.0.0", name, type: "mapping", files: [inspect.file_kind] }),
    detect: { required_columns: inspect.header.filter((h) => columns.has(h)) },
    input: compact({
      delimiter: draft.options.delimiter,
      encoding: draft.options.encoding,
      skip_rows: draft.options.skip_rows,
      sheet: draft.options.sheet,
      decimal: draft.decimal,
      thousands: draft.thousands,
      date_format: draft.dateFormat,
    }),
    account: { hint: source(draft.account) ?? { const: "" } },
    rules: draft.rules.filter((r) => r.column).map((r) => ({ column: r.column, [r.op]: r.value, kind: r.kind })),
    fields,
  };
}
