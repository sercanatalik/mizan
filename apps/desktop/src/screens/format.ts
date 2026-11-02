const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/** `2026-10-31` → `31 Oct 2026`, independent of the system locale. */
export function longDate(iso: string): string {
  const [y, m, d] = iso.split("-");
  return `${Number(d)} ${MONTHS[Number(m) - 1] ?? m} ${y}`;
}

/** A plain decimal from Rust, e.g. `-1234.50`, with thousands separators and a minus sign: `−1,234.50`. */
export function amount(plain: string): string {
  const neg = plain.startsWith("-");
  const [int, frac] = (neg ? plain.slice(1) : plain).split(".");
  const grouped = (int ?? "").replace(/\B(?=(\d{3})+(?!\d))/g, ",");
  return `${neg ? "−" : ""}${grouped}${frac !== undefined ? `.${frac}` : ""}`;
}

/** Labels for asset class keys from the class hint. */
export const CLASS_LABELS: Record<string, string> = {
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

/**
 * A large amount in short form: `279949.48` → `279.95K`, `4812350` → `4.81M`.
 * Below a thousand it keeps all digits. Plus and minus signs are kept.
 */
export function compact(plain: string): string {
  const n = Number(plain);
  if (!Number.isFinite(n)) return plain;
  const sign = plain.startsWith("-") || plain.startsWith("−") ? "−" : plain.startsWith("+") ? "+" : "";
  const a = Math.abs(n);
  if (a >= 1e9) return `${sign}${(a / 1e9).toFixed(2)}B`;
  if (a >= 1e6) return `${sign}${(a / 1e6).toFixed(2)}M`;
  if (a >= 1e3) return `${sign}${(a / 1e3).toFixed(2)}K`;
  return `${sign}${amount(a.toFixed(2))}`;
}
