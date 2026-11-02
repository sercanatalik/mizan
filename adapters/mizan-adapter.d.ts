// Mizan adapter API for script adapters (adapters/<id>/script.ts).
//
// A script exports `detect` and `run`. It runs in a sandbox: no network, no
// file system, no clock (`Date.now()` and `new Date()` throw; `new Date(y, m, d)`
// works), no `Math.random`, 64 MB of memory and 10 seconds per call. Scripts
// cannot import modules; everything they need is in `input` and `mizan`.
// Decimals are always strings ("1234.56"), dates are "YYYY-MM-DD".

/** One row of a CSV or XLSX file, with its 1-based row number in the source. */
interface MizanRow {
  row: number;
  cells: string[];
}

interface MizanInput {
  fileName: string;
  kind: "csv" | "xlsx" | "pdf" | "xml" | "ofx" | "text";
  /** CSV/XLSX: the header row first, then data rows (blank rows skipped). */
  rows?: MizanRow[];
  /** PDF: the text of each page. */
  pages?: string[];
  /** Other text files: the decoded text. */
  text?: string;
}

interface MizanMoney {
  amount: string;
  currency: string;
}

interface MizanInstrument {
  isin?: string;
  cusip?: string;
  figi?: string;
  symbol?: string;
  mic?: string;
  name?: string;
  class_hint?: "equity" | "etf" | "fund" | "bond" | "option" | "crypto" | "commodity" | "cash" | "other";
  currency?: string;
  multiplier?: string;
}

type MizanKind =
  | "deposit" | "withdrawal" | "fx_conversion" | "internal_transfer"
  | "buy" | "sell" | "short_sell" | "buy_to_cover"
  | "dividend" | "interest" | "distribution" | "reward"
  | "fee" | "tax" | "position_in" | "position_out"
  | "split" | "exchange" | "spinoff" | "return_of_capital"
  | "option_exercise" | "option_assignment" | "option_expiry" | "adjustment";

/** A `mizan.import/1` record without `source`; Mizan adds provenance from `locator`. */
interface MizanRecord {
  locator: string;
  external_id?: string;
  account: { hint: string; name_hint?: string };
  kind: MizanKind;
  date: string;
  settle_date?: string;
  instrument?: MizanInstrument;
  quantity?: string;
  price?: MizanMoney;
  gross?: MizanMoney;
  charges?: { type: "commission" | "exchange_fee" | "stamp_duty" | "transaction_tax" | "withholding_tax" | "other"; amount: MizanMoney }[];
  net_cash?: MizanMoney;
  memo?: string;
}

interface MizanAssertion {
  locator: string;
  account: { hint: string };
  as_of: string;
  cash?: MizanMoney[];
  positions?: { instrument: MizanInstrument; quantity: string }[];
}

interface MizanDiagnostic {
  severity: "info" | "warning" | "error";
  message: string;
  locator: string;
}

interface MizanOutput {
  records: MizanRecord[];
  assertions?: MizanAssertion[];
  diagnostics?: MizanDiagnostic[];
}

/** Host helpers. All are pure. */
declare const mizan: {
  /** Parse a locale-formatted number ("1.234,56" with { decimal: ",", thousands: "." }) to a decimal string. Throws if invalid. */
  decimal(text: string, options?: { decimal?: string; thousands?: string }): string;
  /** Parse a date with a strftime-style format ("%d.%m.%Y") to "YYYY-MM-DD". Throws if invalid. */
  date(text: string, format: string): string;
  /** Add an info message to the import review. */
  log(message: string): void;
};
