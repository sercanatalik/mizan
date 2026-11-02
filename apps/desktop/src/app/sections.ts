// Sidebar sections. Order and ids match the native View menu (src-tauri/src/menu.rs).
export type SectionId =
  | "overview"
  | "holdings"
  | "activity"
  | "performance"
  | "income"
  | "tax"
  | "imports"
  | "accounts"
  | "assets"
  | "settings";

export interface Section {
  id: SectionId;
  label: string;
  path: string;
  group: "Portfolio" | "Data" | null;
  icon: string;
}

export const SECTIONS: Section[] = [
  { id: "overview", label: "Overview", path: "/", group: "Portfolio", icon: "M3 13h8V3H3zM13 21h8V11h-8zM3 21h8v-6H3zM13 9h8V3h-8z" },
  { id: "holdings", label: "Holdings", path: "/holdings", group: "Portfolio", icon: "M4 6h16M4 12h16M4 18h10" },
  { id: "activity", label: "Activity", path: "/activity", group: "Portfolio", icon: "M3 12h4l3-8 4 16 3-8h4" },
  { id: "performance", label: "Performance", path: "/performance", group: "Portfolio", icon: "M3 17l6-6 4 4 8-8M15 7h6v6" },
  { id: "income", label: "Income", path: "/income", group: "Portfolio", icon: "M12 3v18M17 7H9.5a3 3 0 0 0 0 6h5a3 3 0 0 1 0 6H6" },
  { id: "tax", label: "Tax", path: "/tax", group: "Portfolio", icon: "M6 3h12v18l-3-2-3 2-3-2-3 2zM9 8h6M9 12h6M9 16h3" },
  { id: "imports", label: "Imports", path: "/imports", group: "Data", icon: "M12 3v12M7 10l5 5 5-5M4 21h16" },
  { id: "accounts", label: "Accounts", path: "/accounts", group: "Data", icon: "M3 10l9-6 9 6M5 10v9M19 10v9M9 10v9M15 10v9M3 21h18" },
  { id: "assets", label: "Assets", path: "/assets", group: "Data", icon: "M20.6 13.4l-7.2 7.2a2 2 0 0 1-2.8 0L3 13V3h10l7.6 7.6a2 2 0 0 1 0 2.8zM7.5 7.5h.01" },
  {
    id: "settings",
    label: "Settings",
    path: "/settings",
    group: null,
    icon: "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM19 12h2M3 12h2M12 3v2M12 19v2M17 7l1.4-1.4M5.6 18.4L7 17M17 17l1.4 1.4M5.6 5.6L7 7",
  },
];

export function sectionPath(id: string): string | undefined {
  return SECTIONS.find((s) => s.id === id)?.path;
}
