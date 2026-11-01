import { screen, within } from "@testing-library/react";
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
  ...p,
});

const holdings: Holdings = {
  accounts: [
    {
      account_id: 1,
      name: "Interactive Brokers",
      positions: [
        pos({ instrument_key: "isin:US0378331005", symbol: "AAPL", name: "Apple Inc.", class: "equity", currency: "USD", quantity: "40",
          cost: { state: "known", total: "6081.60", average: "152.04", currency: "USD", unknown_units: null } }),
        pos({ instrument_key: "isin:SH", symbol: "SH", class: "equity", currency: "USD", quantity: "-8",
          cost: { state: "short", total: null, average: null, currency: null, unknown_units: null } }),
      ],
      cash: [{ currency: "USD", amount: "2417.16" }],
    },
    {
      account_id: 2,
      name: "Degiro",
      positions: [
        pos({ instrument_key: "isin:IE00BK5BQT80", name: "Vanguard FTSE All-World", class: "etf", currency: "EUR", quantity: "20",
          cost: { state: "unknown_basis", total: null, average: null, currency: null, unknown_units: "20" } }),
        pos({ instrument_key: "isin:NL0010273215", name: "ASML Holding", class: "equity", currency: "EUR", quantity: "10",
          check_flag: { statement_quantity: "12", computed_quantity: "10", as_of: "2026-09-30", batch_id: 4 },
          cost: { state: "mixed_currency", total: null, average: null, currency: "EUR", unknown_units: null } }),
      ],
      cash: [],
    },
  ],
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
  expect(await screen.findByText("4 positions in 2 accounts")).toBeInTheDocument();
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

test("no value, weight or gain columns without market data", async () => {
  renderApp("/holdings");
  await screen.findByText("AAPL");
  const headers = screen.getAllByRole("columnheader").map((h) => h.textContent ?? "");
  expect(headers.some((h) => /value|weight|gain|unrealized/i.test(h))).toBe(false);
  expect(screen.getByRole("note")).toHaveTextContent("appear once market data is available");
});
