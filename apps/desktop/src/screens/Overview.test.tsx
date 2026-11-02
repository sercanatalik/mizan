import { act, screen, waitFor, within } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Overview } from "../bridge/generated/Overview";
import { EMPTY, invokeFrom, renderApp } from "../test/render";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

function serve(o: Overview, extra: Record<string, unknown> = {}) {
  vi.mocked(invoke).mockImplementation(invokeFrom({ ...EMPTY, cmd_overview: o, ...extra }) as typeof invoke);
}

beforeEach(() => {
  vi.mocked(invoke).mockReset();
  vi.mocked(listen).mockImplementation((async () => () => {}) as unknown as typeof listen);
});

const t = (amount: string, excluded_no_price = 0) => ({ currency: "TRY", amount, excluded_no_price, excluded_no_rate: 0, oldest_price: "2026-10-14" });

const overview: Overview = {
  base_currency: "TRY",
  total: t("526962.60", 1),
  has_prices: false,
  accounts: [
    { account_id: 1, name: "Interactive Brokers", activity_count: 64, latest_check: { as_of: "2026-10-31", passed: true, failed: [], batch_id: 3 }, total: t("438873.00") },
    {
      account_id: 2,
      name: "Degiro",
      activity_count: 51,
      latest_check: {
        as_of: "2026-09-30",
        passed: false,
        batch_id: 4,
        failed: [
          { label: "isin:NL0010273215", display: "ASML", expected: "12", computed: "10" },
          { label: "EUR", display: "cash", expected: "2366.20", computed: "3650.80" },
        ],
      },
      total: t("88089.60", 1),
    },
    { account_id: 3, name: "Savings", activity_count: 1, latest_check: null, total: t("0.00") },
  ],
  attention: [
    { type: "pending_decisions", count: 2 },
    {
      type: "failed_checks",
      account_id: 2,
      account_name: "Degiro",
      as_of: "2026-09-30",
      failed: [{ label: "isin:NL0010273215", display: "ASML", expected: "12", computed: "10" }],
    },
    { type: "unknown_basis", account_id: 2, account_name: "Degiro", instrument_key: "isin:IE00BK5BQT80", display: "VWCE", units: "20" },
  ],
};

test("without prices the value area asks for a refresh and shows no figures", async () => {
  serve(overview);
  renderApp("/");
  const value = await screen.findByRole("region", { name: "Portfolio value" });
  expect(value).toHaveTextContent("Prices needed");
  expect(value.textContent).not.toMatch(/\d/);
  expect(screen.queryAllByTestId("account-total")).toHaveLength(0);
});

test("with prices: base total, oldest price date, exclusions and account totals", async () => {
  serve({ ...overview, has_prices: true });
  renderApp("/");
  expect(await screen.findByTestId("portfolio-total")).toHaveTextContent("526962.60 TRY");
  const value = screen.getByRole("region", { name: "Portfolio value" });
  expect(value).toHaveTextContent("Prices as of 14 Oct 2026 or later");
  expect(value).toHaveTextContent("excludes 1 position without a price");
  expect(screen.getAllByTestId("account-total").map((e) => e.textContent)).toEqual(["438873.00 TRY", "88089.60 TRY", "0.00 TRY"]);
  expect(screen.queryByRole("region", { name: "Value chart" })).not.toBeInTheDocument();
});

test("background refresh shows an indicator", async () => {
  const handlers: Record<string, (e: { payload: unknown }) => void> = {};
  vi.mocked(listen).mockImplementation((async (name: string, h: (e: { payload: unknown }) => void) => {
    handlers[name] = h;
    return () => {};
  }) as unknown as typeof listen);
  serve({ ...overview, has_prices: true });
  renderApp("/");
  await screen.findByTestId("portfolio-total");
  await waitFor(() => expect(handlers["market:refresh-started"]).toBeDefined());
  act(() => handlers["market:refresh-started"]!({ payload: null }));
  expect(await screen.findByRole("button", { name: "Refreshing prices…" })).toBeDisabled();
  act(() => handlers["market:refresh-finished"]!({ payload: null }));
  expect(await screen.findByRole("button", { name: "Refresh prices" })).toBeEnabled();
});

test("accounts show passing, failing and no-check status", async () => {
  serve(overview);
  renderApp("/");
  const rows = await screen.findAllByTestId("account-row");
  expect(rows[0]).toHaveTextContent("Statement to 31 Oct 2026 · all balances match");
  expect(rows[1]).toHaveTextContent("2 balance checks failed · ASML, cash");
  expect(within(rows[1]!).getByRole("img", { name: "Warning" })).toBeInTheDocument();
  expect(rows[2]).toHaveTextContent("No balance check yet");
});

test("needs-attention items link to where they are fixed", async () => {
  serve(overview);
  renderApp("/");
  const items = await screen.findAllByTestId("attention-item");
  expect(items[0]).toHaveTextContent("2 records need your decision");
  expect(within(items[0]!).getByRole("link", { name: "Review" })).toHaveAttribute("href", "/imports");
  expect(items[1]).toHaveTextContent("Degiro balance checks failed");
  expect(items[1]).toHaveTextContent("ASML: statement 12, Mizan 10");
  expect(items[2]).toHaveTextContent("Unknown cost basis · 20 VWCE in Degiro");
  expect(within(items[2]!).getByRole("link")).toHaveAttribute("href", "/holdings");
});

test("everything in order", async () => {
  serve({ ...overview, accounts: [overview.accounts[0]!], attention: [] });
  renderApp("/");
  expect(await screen.findByText("Everything is in order.")).toBeInTheDocument();
});

const points = [
  { date: "2026-10-30", value: "1000.00", contributions: "1000.00", estimated: true },
  { date: "2026-10-31", value: null, contributions: "1000.00", estimated: false },
  { date: "2026-11-01", value: "1100.00", contributions: "1000.00", estimated: false },
];
const running = {
  cmd_backfill_status: {
    ...EMPTY.cmd_backfill_status,
    sources: [{ ...EMPTY.cmd_backfill_status.sources[0], state: "running", done: 34, planned: 120 }, EMPTY.cmd_backfill_status.sources[1]],
  },
};

test("value over time: chart with value and contributions", async () => {
  serve({ ...overview, has_prices: true }, { cmd_value_history: { base_currency: "TRY", points, valued_days: 2 } });
  renderApp("/");
  const card = await screen.findByRole("region", { name: "Value chart" });
  const chart = within(card).getByRole("img");
  expect(chart).toHaveAccessibleName(/Value: 1100\.00 TRY at the end; Net contributions: 1000\.00 TRY at the end/);
  expect(within(card).getByRole("list", { name: /legend/ })).toHaveTextContent("Estimated values");
  expect(within(card).queryByRole("note")).not.toBeInTheDocument();
});

test("value over time while history downloads", async () => {
  serve({ ...overview, has_prices: true }, { cmd_value_history: { base_currency: "TRY", points, valued_days: 2 }, ...running });
  renderApp("/");
  const note = await screen.findByRole("note");
  expect(note).toHaveTextContent("Price history is still downloading (TEFAS 34 of 120 months)");
});

test("value over time before any history", async () => {
  serve({ ...overview, has_prices: true }, { cmd_value_history: { base_currency: "TRY", points: points.slice(2), valued_days: 0 }, ...running });
  renderApp("/");
  const card = await screen.findByRole("region", { name: "Value chart" });
  expect(card).toHaveTextContent("The value history appears once price history has been downloaded");
  expect(card).toHaveTextContent("TEFAS: 34 of 120 months");
});
