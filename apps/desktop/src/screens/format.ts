const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/** `2026-10-31` → `31 Oct 2026`, independent of the system locale. */
export function longDate(iso: string): string {
  const [y, m, d] = iso.split("-");
  return `${Number(d)} ${MONTHS[Number(m) - 1] ?? m} ${y}`;
}
