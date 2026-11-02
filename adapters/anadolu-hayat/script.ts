// Anadolu Hayat Emeklilik "Geçmiş Fon İşlemleri" (fund transaction history).
//
// Layout: a title row "Beh No <contract> Geçmiş Fon İşlemleri", a header row,
// then one row per fund transaction, newest first. Amounts look like
// "1.234,56 Adet" and "1.234,56 TL"; dates are dd.MM.yyyy; "-" marks an
// empty cell. Rows sharing a date and a description form one event:
//   - contributions: a deposit of buys − sells, then the trades; the state
//     contribution fund (AET) goes to "<contract>-DK" with its own deposit;
//   - fund switches: the trades only, with a warning when the sides differ;
//   - allocation changes: a note, nothing to import.

const HEADER = [
  "İşlem Tarihi", "İşlem Türü", "Fon Adı", "Fon Adedi", "Fon Tutarı",
  "İşlem Tamamlanma Tarihi", "Değişiklik Kapsamı", "Durum", "Açıklama",
];
const TITLE = /^Beh No\s+(\S+)\s+Geçmiş Fon İşlemleri\s*$/;
const CONTRIBUTIONS = ["Katkı Payı Yatırıma Yönlendirme", "Ek Katkı Payı Yatırıma Yönlendirme"];
const SWITCH = "Fon Değişikliği";
const STATE_FUND = "AET";
const SWITCH_TOLERANCE_KURUS = 5;
const TR = { decimal: ",", thousands: "." };

interface Row {
  row: number;
  date: string;
  settle?: string;
  side: "buy" | "sell";
  symbol: string;
  name: string;
  units: string;
  kurus: number;
  description: string;
}

function fail(message: string, locator: string): never {
  throw Object.assign(new Error(message), { locator });
}

function headerMatches(cells: string[] | undefined): boolean {
  return !!cells && HEADER.every((h, i) => (cells[i] ?? "").trim() === h);
}

function contractOf(input: MizanInput): string | undefined {
  const first = input.rows?.[0]?.cells[0] ?? "";
  return TITLE.exec(first.trim())?.[1];
}

/** "1.234,56 TL" → "1234.56" */
function amount(text: string, suffix: string, locator: string): string {
  const t = text.trim();
  if (!t.endsWith(suffix)) fail(`expected an amount in ${suffix}, got "${text}"`, locator);
  return mizan.decimal(t.slice(0, -suffix.length).trim(), TR);
}

/** "1234.56" → 123456 (TL amounts always have at most two decimals). */
function toKurus(dec: string): number {
  const neg = dec.startsWith("-");
  const [whole, frac = ""] = (neg ? dec.slice(1) : dec).split(".");
  const k = Number(whole) * 100 + Number((frac + "00").slice(0, 2));
  return neg ? -k : k;
}

function fromKurus(k: number): string {
  const sign = k < 0 ? "-" : "";
  const a = Math.abs(k);
  return `${sign}${Math.floor(a / 100)}.${String(a % 100).padStart(2, "0")}`;
}

function tl(k: number): MizanMoney {
  return { amount: fromKurus(Math.abs(k)), currency: "TRY" };
}

function locatorOf(rows: Row[]): string {
  const ns = rows.map((r) => r.row).sort((a, b) => a - b);
  return ns.length === 1 ? `row ${ns[0]}` : `rows ${ns.join(", ")}`;
}

export function detect(input: MizanInput): number {
  if (input.kind !== "xlsx" || !input.rows) return 0;
  if (headerMatches(input.rows[1]?.cells)) return contractOf(input) ? 95 : 80;
  return 0;
}

export function run(input: MizanInput): MizanOutput {
  const rows = input.rows ?? [];
  const contract = contractOf(input);
  if (!contract) fail('the title row "Beh No … Geçmiş Fon İşlemleri" is missing', "row 1");
  if (!headerMatches(rows[1]?.cells)) fail("unexpected column headers", `row ${rows[1]?.row ?? 2}`);

  const main = { hint: contract, name_hint: "Anadolu Hayat Emeklilik" };
  const state = { hint: `${contract}-DK`, name_hint: "Anadolu Hayat Emeklilik (Devlet Katkısı)" };
  const records: MizanRecord[] = [];
  const diagnostics: MizanDiagnostic[] = [];

  // The export is newest first; read it oldest first.
  const parsed: Row[] = [];
  for (const { row, cells } of rows.slice(2).reverse()) {
    const locator = `row ${row}`;
    const [date, type, fund, units, value, completed, , status, description] = cells.map((c) => (c ?? "").trim());
    if (type === "Fon Dağılım Değişikliği") {
      diagnostics.push({ severity: "info", message: `fund allocation changed on ${mizan.date(date, "%d.%m.%Y")}; nothing to import`, locator });
      continue;
    }
    if (type !== "Fon Alışı" && type !== "Fon Satışı") {
      diagnostics.push({ severity: "warning", message: `unknown transaction type "${type}" skipped`, locator });
      continue;
    }
    if (status !== "Tamamlandı") {
      diagnostics.push({ severity: "warning", message: `transaction not completed yet ("${status}") skipped`, locator });
      continue;
    }
    const code = /\(([A-Z0-9]{2,5})\)\s*$/.exec(fund);
    if (!code) fail(`no fund code in "${fund}"`, locator);
    parsed.push({
      row,
      date: mizan.date(date, "%d.%m.%Y"),
      settle: completed && completed !== "-" ? mizan.date(completed, "%d.%m.%Y") : undefined,
      side: type === "Fon Alışı" ? "buy" : "sell",
      symbol: code[1],
      name: fund.slice(0, code.index).trim(),
      units: amount(units, "Adet", locator),
      kurus: toKurus(amount(value, "TL", locator)),
      description,
    });
  }

  // Group by (date, description), keeping first-seen order.
  const groups = new Map<string, Row[]>();
  for (const r of parsed) {
    const key = `${r.date}|${r.description}`;
    const g = groups.get(key);
    if (g) g.push(r);
    else groups.set(key, [r]);
  }

  const trade = (r: Row, account: MizanRecord["account"]): MizanRecord => {
    const rec: MizanRecord = {
      locator: `row ${r.row}`,
      account,
      kind: r.side,
      date: r.date,
      instrument: { symbol: r.symbol, name: r.name, class_hint: "fund", currency: "TRY" },
      quantity: r.units,
      net_cash: tl(r.kurus),
    };
    if (r.settle) rec.settle_date = r.settle;
    return rec;
  };
  const net = (g: Row[]) => g.reduce((s, r) => s + (r.side === "buy" ? r.kurus : -r.kurus), 0);

  for (const g of groups.values()) {
    const { date, description } = g[0];
    if (CONTRIBUTIONS.includes(description)) {
      const parts: [MizanRecord["account"], Row[]][] = [
        [main, g.filter((r) => r.symbol !== STATE_FUND)],
        [state, g.filter((r) => r.symbol === STATE_FUND)],
      ];
      for (const [account, part] of parts) {
        if (part.length === 0) continue;
        const k = net(part);
        if (k !== 0) {
          records.push({
            locator: locatorOf(part),
            account,
            kind: k > 0 ? "deposit" : "withdrawal",
            date,
            net_cash: tl(k),
            memo: description,
          });
        }
        for (const r of part) records.push(trade(r, account));
      }
    } else {
      for (const r of g) records.push(trade(r, r.symbol === STATE_FUND ? state : main));
      const k = net(g);
      if (description !== SWITCH) {
        diagnostics.push({ severity: "warning", message: `unknown description "${description}": trades imported without a deposit`, locator: locatorOf(g) });
      } else if (Math.abs(k) > SWITCH_TOLERANCE_KURUS) {
        const sells = g.filter((r) => r.side === "sell").reduce((s, r) => s + r.kurus, 0);
        diagnostics.push({
          severity: "warning",
          message: `fund switch on ${date} is unbalanced: sells ${fromKurus(sells)} TL, buys ${fromKurus(sells + k)} TL, difference ${fromKurus(Math.abs(k))} TL`,
          locator: locatorOf(g),
        });
      }
    }
  }

  return { records, diagnostics };
}
