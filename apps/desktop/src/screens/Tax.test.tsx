import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import type { Country } from "../bridge/generated/Country";
import type { GainRowView } from "../bridge/generated/GainRowView";
import type { GainSection } from "../bridge/generated/GainSection";
import type { TaxReport } from "../bridge/generated/TaxReport";
import type { TaxYears } from "../bridge/generated/TaxYears";
import { EMPTY, invokeFrom, renderApp } from "../test/render";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
const mockInvoke = vi.mocked(invoke);
const mockOpen = vi.mocked(open);

beforeEach(() => {
  vi.mocked(listen).mockResolvedValue(() => {});
});

const NOTICE = "This is a working paper to help you prepare your tax return, not tax advice.";

const row = (extra: Partial<GainRowView> = {}): GainRowView => ({
  description: "Apple Inc. (AAPL)",
  symbol: "AAPL",
  isin: "US0378331005",
  account: "Broker",
  date_acquired: "2023-03-01",
  date_sold: "2025-06-10",
  quantity: "10",
  proceeds: "2200.00",
  cost: "1500.00",
  indexed_cost: null,
  adjustment_code: null,
  adjustment: null,
  gain: "700.00",
  rule: null,
  term: null,
  reason: null,
  notes: [],
  ...extra,
});

const totals = (net: string) => ({ disposals: 1, proceeds: "0.00", cost: "0.00", adjustments: "0.00", gains: net, losses: "0.00", net });

const section = (id: string, title: string, rows: GainRowView[], counted = true): GainSection => ({
  id,
  title,
  note: null,
  counted,
  rows,
  totals: totals("700.00"),
});

function report(country: Country, extra: Partial<TaxReport> = {}): TaxReport {
  const currency = { uk: "GBP", us: "USD", tr: "TRY" }[country];
  return {
    country,
    year: country === "uk" ? "2025-26" : "2025",
    first_day: country === "uk" ? "2025-04-06" : "2025-01-01",
    last_day: country === "uk" ? "2026-04-05" : "2025-12-31",
    currency,
    notice: NOTICE,
    computed_at: "2026-11-02T10:00:00Z",
    rate_source: "TCMB forex buying rates on each transaction date",
    not_computed: [],
    incomplete: false,
    summary: [
      { label: "Disposals", value: "2" },
      { label: "Gains", value: "600.00" },
      { label: "Losses", value: "250.00" },
      { label: "Net gain", value: "350.00" },
    ],
    gains: [section("disposals", "Disposals", [row({ rule: "pool", gain: "600.00" }), row({ rule: "30 days", gain: "-250.00" })])],
    income: [{ id: "income", title: "Dividends and interest", note: null, counted: true, rows: [], totals: [] }],
    notes: [],
    accounts: [
      { id: 1, name: "Broker", treatment: "taxable" },
      { id: 2, name: "Anadolu Hayat", treatment: "pension" },
    ],
    excluded: [{ id: 2, name: "Anadolu Hayat", treatment: "pension" }],
    has_activity: true,
    ...extra,
  };
}

const years = (country: Country, labels: string[], def: string | null): TaxYears => ({
  country,
  years: labels.map((label) => ({ label, first_day: "2025-04-06", last_day: "2026-04-05" })),
  default: def,
  current: country === "uk" ? "2026-27" : "2026",
});

function serve(reports: Partial<Record<Country, TaxReport>>, yearsBy: Partial<Record<Country, TaxYears>> = {}, extra: Record<string, unknown> = {}) {
  mockInvoke.mockImplementation(
    invokeFrom({
      ...EMPTY,
      cmd_tax_years: (a: unknown) => {
        const c = (a as { country: Country }).country;
        return yearsBy[c] ?? years(c, c === "uk" ? ["2025-26", "2024-25"] : ["2025", "2024"], c === "uk" ? "2025-26" : "2025");
      },
      cmd_tax_report: (a: unknown) => reports[(a as { country: Country }).country] ?? report((a as { country: Country }).country),
      cmd_set_tax_treatment: null,
      cmd_export_tax_csv: { gains: "/Users/a/Desktop/mizan-uk-2025-26-gains.csv", income: "/Users/a/Desktop/mizan-uk-2025-26-income.csv" },
      ...extra,
    }) as typeof invoke,
  );
}

test("UK report shows the notice, summary and matching rules", async () => {
  serve({});
  renderApp("/tax");
  expect(await screen.findByTestId("tax-notice")).toHaveTextContent("not tax advice");
  expect(screen.getByTestId("figure-Disposals")).toHaveTextContent("2");
  expect(screen.getByTestId("figure-Net gain")).toHaveTextContent("350.00 GBP");
  const rows = screen.getAllByTestId("gain-row");
  expect(rows[0]).toHaveTextContent("pool");
  expect(rows[1]).toHaveTextContent("30 days");
  expect(rows[1]).toHaveTextContent("−250.00");
  expect(screen.queryByText(/tax owed/i)).toBeNull();
  expect(mockInvoke).toHaveBeenCalledWith("cmd_tax_report", { country: "uk", year: "2025-26" });
});

test("US report groups short and long terms and marks wash sales", async () => {
  serve({
    us: report("us", {
      gains: [
        section("short", "Short-term (held one year or less)", [row({ term: "short", adjustment_code: "W", adjustment: "200.00", gain: "0.00", proceeds: "800.00", cost: "1000.00" })]),
        section("long", "Long-term (held more than one year)", [row({ term: "long" })]),
      ],
      notes: ["The Form 8949 box (A to F) depends on what your broker reported to the IRS; Mizan does not choose it.", "Whether dividends are qualified is not determined."],
    }),
  });
  renderApp("/tax");
  await userEvent.click(await screen.findByRole("button", { name: "US" }));
  const short = await screen.findByRole("region", { name: "Short-term (held one year or less)" });
  expect(within(short).getByTestId("adjustment-code")).toHaveTextContent("W");
  expect(within(short).getByRole("columnheader", { name: "(f) Code" })).toBeInTheDocument();
  expect(screen.getByRole("region", { name: "Long-term (held more than one year)" })).toBeInTheDocument();
  expect(screen.getByTestId("tax-notes")).toHaveTextContent("qualified");
});

test("Turkish report lists taxed-at-source holdings apart and shows indexed costs", async () => {
  serve({
    tr: report("tr", {
      gains: [
        section("declared", "Foreign securities", [row({ cost: "28350.00", indexed_cost: "51030.00", proceeds: "85800.00", gain: "34770.00" })]),
        { ...section("taxed_at_source", "Taxed at source", [row({ description: "AFT", isin: null })], false), note: "Turkish funds and Borsa Istanbul securities are taxed at source and are not added to the totals." },
        section("origin_unknown", "Origin unknown", [], false),
      ],
    }),
  });
  renderApp("/tax");
  await userEvent.click(await screen.findByRole("button", { name: "Turkey" }));
  const declared = await screen.findByRole("region", { name: "Foreign securities" });
  expect(declared).toHaveTextContent("51,030.00");
  expect(declared).toHaveTextContent("34,770.00");
  const source = screen.getByRole("region", { name: "Taxed at source" });
  expect(within(source).getByTestId("section-total")).toHaveTextContent("Not counted");
  // An empty, uncounted section is not shown.
  expect(screen.queryByRole("region", { name: "Origin unknown" })).toBeNull();
});

test("rows not computed are listed with their reasons", async () => {
  serve({
    uk: report("uk", {
      incomplete: true,
      not_computed: [
        { reason: "cost unknown", count: 1 },
        { reason: "no exchange rate", count: 1 },
      ],
      gains: [section("disposals", "Disposals", [row({ reason: "cost unknown", gain: null, cost: null }), row({ reason: "no exchange rate", gain: null, proceeds: null })])],
    }),
  });
  renderApp("/tax");
  expect(await screen.findByTestId("not-computed")).toHaveTextContent("2 rows not computed: cost unknown (1) · no exchange rate (1)");
  expect(screen.getAllByTestId("row-reason").map((r) => r.textContent)).toEqual(["cost unknown", "no exchange rate"]);
});

test("no taxable activity shows an empty state that links to Imports, and accounts stay editable", async () => {
  serve({ uk: report("uk", { has_activity: false, gains: [], income: [], summary: [] }) }, { uk: years("uk", [], null) });
  renderApp("/tax");
  expect(await screen.findByText("No taxable activity yet")).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Go to Imports" })).toHaveAttribute("href", "/imports");
  expect(screen.queryByTestId("summary")).toBeNull();
  expect(mockInvoke).toHaveBeenCalledWith("cmd_tax_report", { country: "uk", year: "2026-27" });
  expect(await screen.findByRole("combobox", { name: "Tax treatment of Anadolu Hayat" })).toHaveValue("pension");
});

test("changing an account's treatment saves it", async () => {
  serve({});
  renderApp("/tax");
  await userEvent.selectOptions(await screen.findByRole("combobox", { name: "Tax treatment of Broker" }), "isa");
  expect(mockInvoke).toHaveBeenCalledWith("cmd_set_tax_treatment", { account_id: 1, treatment: "isa" });
});

test("picking another year asks for that year's report", async () => {
  serve({});
  renderApp("/tax");
  await userEvent.selectOptions(await screen.findByRole("combobox", { name: "Tax year" }), "2024-25");
  expect(mockInvoke).toHaveBeenCalledWith("cmd_tax_report", { country: "uk", year: "2024-25" });
});

test("export writes nothing when the folder picker is cancelled", async () => {
  serve({});
  mockOpen.mockResolvedValueOnce(null);
  renderApp("/tax");
  await userEvent.click(await screen.findByRole("button", { name: "Export CSV" }));
  expect(mockOpen).toHaveBeenCalled();
  expect(mockInvoke).not.toHaveBeenCalledWith("cmd_export_tax_csv", expect.anything());
});

test("export writes both files to the chosen folder", async () => {
  serve({});
  mockOpen.mockResolvedValueOnce("/Users/a/Desktop");
  renderApp("/tax");
  await userEvent.click(await screen.findByRole("button", { name: "Export CSV" }));
  expect(mockInvoke).toHaveBeenCalledWith("cmd_export_tax_csv", { country: "uk", year: "2025-26", dir: "/Users/a/Desktop" });
  expect(await screen.findByTestId("export-done")).toHaveTextContent("Saved mizan-uk-2025-26-gains.csv and mizan-uk-2025-26-income.csv");
});
