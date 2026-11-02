// Sidebar sections. The native Go menu (src-tauri/src/menu.rs) mirrors ids and order:
// `npm run gen:sections` writes src-tauri/sections.json, which a Rust test checks.
export type SectionId =
  | "overview"
  | "inbox"
  | "holdings"
  | "accounts"
  | "performance"
  | "income"
  | "activity"
  | "imports"
  | "adapters"
  | "tax"
  | "exports"
  | "assets"
  | "market-data"
  | "settings";

export type SectionGroup = "Home" | "Portfolio" | "Records" | "Reports" | "Reference";

export interface Section {
  id: SectionId;
  label: string;
  path: string;
  /** Null for entries below the groups (Settings). */
  group: SectionGroup | null;
  icon: string;
  /** Digit for the ⌘ shortcut in the Go menu, for the first nine entries. */
  shortcut: number | null;
}

/** Group labels shown in the sidebar; Home entries sit above them without a label. */
export const LABELED_GROUPS: SectionGroup[] = ["Portfolio", "Records", "Reports", "Reference"];

const S = (id: SectionId, label: string, path: string, group: SectionGroup | null, icon: string): Omit<Section, "shortcut"> => ({
  id,
  label,
  path,
  group,
  icon,
});

const LIST = [
  S("overview", "Overview", "/", "Home", "M3 13h8V3H3zM13 21h8V11h-8zM3 21h8v-6H3zM13 9h8V3h-8z"),
  S("inbox", "Inbox", "/inbox", "Home", "M3 13h5l2 3h4l2-3h5M5 5h14l2 8v6H3v-6z"),
  S("holdings", "Holdings", "/holdings", "Portfolio", "M4 6h16M4 12h16M4 18h10"),
  S("accounts", "Accounts", "/accounts", "Portfolio", "M3 10l9-6 9 6M5 10v9M19 10v9M9 10v9M15 10v9M3 21h18"),
  S("performance", "Performance", "/performance", "Portfolio", "M3 17l6-6 4 4 8-8M15 7h6v6"),
  S("income", "Income", "/income", "Portfolio", "M12 3v18M17 7H9.5a3 3 0 0 0 0 6h5a3 3 0 0 1 0 6H6"),
  S("activity", "Activity", "/activity", "Records", "M3 12h4l3-8 4 16 3-8h4"),
  S("imports", "Imports", "/imports", "Records", "M12 3v12M7 10l5 5 5-5M4 21h16"),
  S("adapters", "Adapters", "/adapters", "Records", "M9 3v6M15 3v6M6 9h12v4a6 6 0 0 1-12 0zM12 19v2"),
  S("tax", "Tax", "/tax", "Reports", "M6 3h12v18l-3-2-3 2-3-2-3 2zM9 8h6M9 12h6M9 16h3"),
  S("exports", "Exports", "/exports", "Reports", "M12 15V3M7 8l5-5 5 5M4 15v6h16v-6"),
  S("assets", "Assets", "/assets", "Reference", "M20.6 13.4l-7.2 7.2a2 2 0 0 1-2.8 0L3 13V3h10l7.6 7.6a2 2 0 0 1 0 2.8zM7.5 7.5h.01"),
  S("market-data", "Market data", "/market-data", "Reference", "M3 3v18h18M7 15l4-4 3 3 5-6"),
  S(
    "settings",
    "Settings",
    "/settings",
    null,
    "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM19 12h2M3 12h2M12 3v2M12 19v2M17 7l1.4-1.4M5.6 18.4L7 17M17 17l1.4 1.4M5.6 5.6L7 7",
  ),
];

export const SECTIONS: Section[] = LIST.map((s, i) => ({ ...s, shortcut: i < 9 ? i + 1 : null }));

export function sectionPath(id: string): string | undefined {
  return SECTIONS.find((s) => s.id === id)?.path;
}

/** The section a pathname belongs to (account pages belong to Accounts). */
export function sectionForPath(pathname: string): Section | undefined {
  if (pathname === "/") return SECTIONS[0];
  return SECTIONS.find((s) => s.path !== "/" && (pathname === s.path || pathname.startsWith(`${s.path}/`)));
}
