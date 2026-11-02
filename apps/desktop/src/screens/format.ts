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
