import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Holdings } from "../bridge/generated/Holdings";
import type { Position } from "../bridge/generated/Position";
import { EMPTY, invokeFrom, renderApp } from "../test/render";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

const pos = (p: Partial<Position> & Pick<Position, "instrument_key" | "quantity">): Position => ({
  asset_id: 1,
  symbol: null,
  name: null,
  isin: null,
  class: "other",
  currency: null,
  check_flag: null,
  cost: { state: "known", total: null, average: null, currency: null, unknown_units: null },
  price: null,
  value: null,
  base_value: null,
  weight: null,
  gain: null,
  value_note: "no price",
  gain_note: null,
  open_lots: 1,
  series_key: null,
  ...p,
});

const total = (amount: string, excluded_no_price = 0) => ({ currency: "TRY", amount, excluded_no_price, excluded_no_rate: 0, oldest_price: null });

const holdings: Holdings = {
  accounts: [
    {
      account_id: 1,
      name: "Interactive Brokers",
      positions: [
        pos({ instrument_key: "isin:US0378331005", symbol: "AAPL", name: "Apple Inc.", class: "equity", currency: "USD", quantity: "40",
          cost: { state: "known", total: "6081.60", average: "152.04", currency: "USD", unknown_units: null },
          price: { value: "160.00", currency: "USD", date: "2026-10-30", stale: false, source: "manual" },
          value: "6400.00", base_value: "318560.00", weight: "84.2", value_note: null, open_lots: 2, series_key: "yahoo:AAPL",
          gain: { amount: "+318.40", currency: "USD", pct: "+5.2%" } }),
        pos({ instrument_key: "isin:SH", symbol: "SH", class: "equity", currency: "USD", quantity: "-8",
          cost: { state: "short", total: null, average: null, currency: null, unknown_units: null } }),
      ],
      cash: [{ currency: "USD", amount: "2417.16", base_value: "120313.00" }],
      total: total("438873.00", 1),
    },
    {
      account_id: 2,
      name: "Degiro",
      positions: [
        pos({ instrument_key: "isin:IE00BK5BQT80", name: "Vanguard FTSE All-World", class: "etf", currency: "EUR", quantity: "20",
          cost: { state: "unknown_basis", total: null, average: null, currency: null, unknown_units: "20" },
          price: { value: "119.04", currency: "EUR", date: "2026-10-14", stale: true, source: "manual" },
          value: "2380.80", base_value: "88089.60", weight: "100.0", value_note: null, gain_note: "not available: unknown cost basis" }),
        pos({ instrument_key: "isin:NL0010273215", name: "ASML Holding", class: "equity", currency: "EUR", quantity: "10",
          check_flag: { statement_quantity: "12", computed_quantity: "10", as_of: "2026-09-30", batch_id: 4 },
          cost: { state: "mixed_currency", total: null, average: null, currency: "EUR", unknown_units: null } }),
      ],
      cash: [],
      total: total("88089.60", 1),
    },
  ],
  base_currency: "TRY",
  total: total("526962.60", 2),
  has_prices: true,
};

beforeEach(() => {
  vi.mocked(invoke).mockReset();
  vi.mocked(invoke).mockImplementation(invokeFrom({ ...EMPTY, cmd_holdings: holdings }) as typeof invoke);
  vi.mocked(listen).mockImplementation((async () => () => {}) as unknown as typeof listen);
});

const groups = () =>
  screen.getAllByTestId("holdings-group").map((g) => within(g).getByRole("rowheader").textContent?.replace(/\s+/g, " ").trim());

test("groups by account, asset class and currency", async () => {
  renderApp("/holdings");
  expect(await screen.findByText("All accounts · 4 positions in 2 accounts")).toBeInTheDocument();
  expect(groups()).toEqual(["Interactive Brokers · 3", "Degiro · 2"]);
  await userEvent.click(screen.getByRole("button", { name: "Asset class" }));
  expect(groups()).toEqual(["Equities · 3", "Cash · 1", "ETFs · 1"]);
  await userEvent.click(screen.getByRole("button", { name: "Currency" }));
  expect(groups()).toEqual(["USD · 3", "EUR · 2"]);
});

test("shows provisional cost or a reason it is not available", async () => {
  renderApp("/holdings");
  const row = async (text: string) => (await screen.findByText(text)).closest("tr")!;
  expect(await row("AAPL")).toHaveTextContent("152.04 USD");
  expect(await row("AAPL")).toHaveTextContent("6081.60 USD");
  expect(await row("SH")).toHaveTextContent("not available for short positions yet");
  expect(await row("SH")).toHaveTextContent("-8");
  expect(await row("Vanguard FTSE All-World")).toHaveTextContent("not available: 20 units with unknown cost basis");
  expect(await row("Vanguard FTSE All-World")).toHaveTextContent("20 units with unknown cost basis");
  expect(await row("ASML Holding")).toHaveTextContent("not available: paid in more than one currency");
  expect(await row("Cash · USD")).toHaveTextContent("2417.16");
  expect(screen.getAllByRole("columnheader").map((h) => h.textContent)).toContain("Average cost (provisional)");
});

test("failed balance check flag links to imports", async () => {
  renderApp("/holdings");
  const link = await screen.findByRole("link", { name: "Statement says 12, see the import" });
  expect(link).toHaveAttribute("href", "/imports");
});

test("priced, stale, unpriced and unknown-basis rows", async () => {
  renderApp("/holdings");
  const row = async (text: string) => (await screen.findByText(text)).closest("tr")!;
  const aapl = await row("AAPL");
  expect(aapl).toHaveTextContent("160.00 USD");
  expect(aapl).toHaveTextContent("as of 30 Oct 2026");
  expect(aapl).toHaveTextContent("6400.00 USD");
  expect(aapl).toHaveTextContent("84.2%");
  expect(aapl).toHaveTextContent("+318.40 USD");
  expect(aapl).toHaveTextContent("+5.2%");
  const vwce = await row("Vanguard FTSE All-World");
  expect(vwce).toHaveTextContent("stale");
  expect(vwce).toHaveTextContent("as of 14 Oct 2026");
  expect(vwce).toHaveTextContent("not available: unknown cost basis");
  expect(await row("SH")).toHaveTextContent("no price");
  const headers = screen.getAllByRole("columnheader").map((h) => h.textContent ?? "");
  for (const h of ["Price (as of)", "Value", "Weight", "Unrealized"]) expect(headers).toContain(h);
  expect(screen.getByRole("note")).toHaveTextContent("Values and weights in TRY");
  expect(screen.getByRole("button", { name: "Refresh prices" })).toBeInTheDocument();
});

test("notice when no asset has a price", async () => {
  vi.mocked(invoke).mockImplementation(invokeFrom({ ...EMPTY, cmd_holdings: { ...holdings, has_prices: false } }) as typeof invoke);
  renderApp("/holdings");
  expect(await screen.findByRole("note")).toHaveTextContent("Values appear once prices are refreshed");
});


const aaplDetail = {
  row: { kind: "price", key: "yahoo:AAPL", name: "AAPL", code: "AAPL", source: "yahoo", currency: "USD", latest: "160.00", latest_date: "2026-10-30", first_date: "2020-01-02", count: 700, stale: false, series_gaps: 0, held_gaps: 0, edits: 0, used: true },
  points: [],
  total: 3,
  offset: 0,
  chart: [
    { period: "2025-06-01", value: "120.00", edited: false },
    { period: "2026-01-02", value: "140.00", edited: false },
    { period: "2026-10-30", value: "160.00", edited: false },
  ],
  manual_only: false,
};

function serveInspector() {
  vi.mocked(invoke).mockImplementation(
    invokeFrom({
      ...EMPTY,
      cmd_holdings: holdings,
      cmd_market_series_detail: (a: unknown) => {
        if ((a as { key: string }).key !== "yahoo:AAPL") throw new Error("unknown series");
        return aaplDetail;
      },
    }) as typeof invoke,
  );
}

test("selecting a position opens the inspector beside the table", async () => {
  serveInspector();
  renderApp("/holdings");
  expect(screen.queryByRole("region", { name: "Inspector" })).toBeNull();
  await userEvent.click(await screen.findByRole("button", { name: "Inspect AAPL in Interactive Brokers" }));
  const insp = screen.getByRole("region", { name: "Inspector" });
  expect(within(insp).getByRole("heading", { name: "AAPL" })).toBeInTheDocument();
  expect(insp).toHaveTextContent("6400.00 USD");
  expect(insp).toHaveTextContent("+318.40 USD (+5.2%)");
  expect(insp).toHaveTextContent("152.04 USD");
  expect(insp).toHaveTextContent("Open lots2");
  const chart = await within(insp).findByRole("img");
  expect(chart).toHaveAccessibleName(/1 Nov 2025|2 Jan 2026 to 30 Oct 2026/);
  const row = screen.getByRole("button", { name: "Inspect AAPL in Interactive Brokers" }).closest("tr")!;
  expect(row).toHaveAttribute("aria-selected", "true");
});

test("a position without prices says so in the inspector", async () => {
  serveInspector();
  renderApp("/holdings");
  await userEvent.click(await screen.findByRole("button", { name: "Inspect ASML Holding in Degiro" }));
  const insp = screen.getByRole("region", { name: "Inspector" });
  expect(insp).toHaveTextContent("No prices are stored for this asset");
});

test("the inspector's Activity link filters to the position", async () => {
  serveInspector();
  const { router } = renderApp("/holdings");
  await userEvent.click(await screen.findByRole("button", { name: "Inspect AAPL in Interactive Brokers" }));
  await userEvent.click(within(screen.getByRole("region", { name: "Inspector" })).getByRole("button", { name: "Activity" }));
  await waitFor(() => expect(router.state.location.pathname).toBe("/activity"));
  expect(router.state.location.search).toBe("?account=1&text=AAPL");
  expect(mockActivityQuery()).toMatchObject({ accounts: [1], text: "AAPL" });
});

test("close and ⌘I toggle the inspector", async () => {
  serveInspector();
  renderApp("/holdings");
  await userEvent.click(await screen.findByRole("button", { name: "Inspect AAPL in Interactive Brokers" }));
  await userEvent.click(screen.getByRole("button", { name: "Close inspector" }));
  expect(screen.queryByRole("region", { name: "Inspector" })).toBeNull();
  await userEvent.keyboard("{Meta>}i{/Meta}");
  expect(screen.getByRole("region", { name: "Inspector" })).toBeInTheDocument();
  await userEvent.keyboard("{Meta>}i{/Meta}");
  expect(screen.queryByRole("region", { name: "Inspector" })).toBeNull();
});

/** The query of the last cmd_activities call. */
function mockActivityQuery() {
  const calls = vi.mocked(invoke).mock.calls.filter((c) => c[0] === "cmd_activities");
  return (calls[calls.length - 1]?.[1] as { query: unknown } | undefined)?.query;
}
