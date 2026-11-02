import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
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
          { label: "isin:NL0010273215", display: "ASML", expected: "12", computed: "10", difference: "2" },
          { label: "EUR", display: "cash", expected: "2366.20", computed: "3650.80", difference: "-1284.6" },
        ],
      },
      total: t("88089.60", 1),
    },
    { account_id: 3, name: "Savings", activity_count: 1, latest_check: null, total: t("0.00") },
  ],
  allocation: {
    rows: [
      { class: "fund", value: "300000.00", share: "60.0" },
      { class: "cash", value: "200000.00", share: "40.0" },
    ],
    excluded: 1,
  },
  attention: [
    { type: "pending_decisions", count: 2 },
    {
      type: "failed_checks",
      account_id: 2,
      account_name: "Degiro",
      as_of: "2026-09-30",
      failed: [{ label: "isin:NL0010273215", display: "ASML", expected: "12", computed: "10", difference: "2" }],
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
  expect(await screen.findByTestId("portfolio-total")).toHaveTextContent("526.96K TRY");
  const value = screen.getByRole("region", { name: "Portfolio value" });
  expect(value).toHaveTextContent("Prices as of 14 Oct 2026 or later");
  expect(value).toHaveTextContent("excludes 1 position without a price");
  expect(screen.getAllByTestId("account-total").map((e) => e.textContent)).toEqual(["438.87K TRY", "88.09K TRY", "0.00 TRY"]);
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

test("an account row opens the account page", async () => {
  serve(overview);
  renderApp("/");
  const rows = await screen.findAllByTestId("account-row");
  const link = within(rows[0]!).getByRole("link");
  expect(link.getAttribute("href")).toMatch(/^\/accounts\/\d+$/);
});

const inboxItem = (key: string, title: string) => ({
  key,
  kind: "check" as const,
  title,
  subtitle: "",
  body: "",
  facts: [],
  check_lines: [],
  since: "2026-10-01",
  action: { type: "import_review" as const },
  fingerprint: "f",
  snoozed_until: null,
});

test("the Inbox preview shows the three newest items and opens the Inbox at one", async () => {
  const items = ["A", "B", "C", "D", "E", "F", "G"].map((x) => inboxItem(`k${x}`, `Item ${x}`));
  serve(overview, { cmd_inbox: { items, snoozed: [] }, cmd_inbox_count: 7 });
  const { router } = renderApp("/");
  const preview = await screen.findByRole("region", { name: "Inbox" });
  const shown = await within(preview).findAllByTestId("inbox-preview-item");
  expect(shown.map((e) => e.textContent)).toEqual(["Item A", "Item B", "Item C"]);
  expect(within(preview).getByRole("link", { name: "Open Inbox · 7" })).toHaveAttribute("href", "/inbox");
  await userEvent.click(shown[1]!);
  await waitFor(() => expect(router.state.location.pathname).toBe("/inbox"));
  expect(router.state.location.state).toEqual({ select: "kB" });
});

test("everything in order", async () => {
  serve({ ...overview, accounts: [overview.accounts[0]!], attention: [] });
  renderApp("/");
  const preview = await screen.findByRole("region", { name: "Inbox" });
  expect(await within(preview).findByText("Everything is in order.")).toBeInTheDocument();
});

test("allocation by asset class with the lines left out", async () => {
  serve({ ...overview, has_prices: true });
  renderApp("/");
  const card = await screen.findByRole("region", { name: "Allocation" });
  const rows = within(card).getAllByTestId("allocation-row");
  expect(rows[0]).toHaveTextContent("Funds");
  expect(rows[0]).toHaveTextContent("300,000.00");
  expect(rows[0]).toHaveTextContent("60.0%");
  expect(rows[1]).toHaveTextContent("Cash");
  expect(within(card).getByText("Leaves out 1 line without a price or exchange rate")).toBeInTheDocument();
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

test("the chart follows the view bar period", async () => {
  const long = [
    { date: "2026-06-01", value: "500.00", contributions: "500.00", estimated: false },
    { date: "2026-08-01", value: "800.00", contributions: "800.00", estimated: false },
    ...points,
  ];
  let stored = { account_id: null as number | null, period: "since_start", real: false };
  serve(
    { ...overview, has_prices: true },
    {
      cmd_value_history: { base_currency: "TRY", points: long, valued_days: 4 },
      cmd_view_get: () => stored,
      cmd_view_set: (a: unknown) => (stored = (a as { view: typeof stored }).view),
    },
  );
  renderApp("/");
  const card = await screen.findByRole("region", { name: "Value chart" });
  expect(within(card).getByRole("img")).toHaveAccessibleName(/1 Jun 2026 to 1 Nov 2026/);
  await userEvent.click(within(screen.getByRole("toolbar", { name: "View" })).getByRole("button", { name: "3M" }));
  await waitFor(() => expect(within(screen.getByRole("region", { name: "Value chart" })).getByRole("img")).toHaveAccessibleName(/30 Oct 2026 to 1 Nov 2026/));
  expect(screen.getByRole("region", { name: "Value chart" })).toHaveTextContent("from 2 Aug 2026");
});

test("a period chosen on Performance is shown on Overview", async () => {
  let stored = { account_id: null as number | null, period: "ytd", real: false };
  serve(
    { ...overview, has_prices: true },
    {
      cmd_view_get: () => stored,
      cmd_view_set: (a: unknown) => (stored = (a as { view: typeof stored }).view),
      cmd_performance: () => {
        throw new Error("not needed");
      },
    },
  );
  const { router } = renderApp("/performance");
  const bar = await screen.findByRole("toolbar", { name: "View" });
  await userEvent.click(within(bar).getByRole("button", { name: "1Y" }));
  await router.navigate("/");
  const again = await screen.findByRole("toolbar", { name: "View" });
  expect(within(again).getByRole("button", { name: "1Y" })).toHaveAttribute("aria-pressed", "true");
});

const fig = (value: string | null, reason: string | null = null) => ({ value, annualized: null, reason, estimated_days: 0 });
function perfView(benchmark: boolean, unavailable: string | null = null) {
  return {
    base_currency: "TRY",
    benchmark_options: [],
    benchmark_asset_id: benchmark ? 7 : null,
    inflation: { label: "HICP Türkiye (Eurostat)", through: unavailable ? null : "Sep 2026", unavailable },
    returns: {
      period: "ytd",
      from: "2026-01-01",
      clamped: false,
      to: "2026-11-01",
      twr: fig("+9.10%"),
      mwr: fig("+8.30%"),
      real_twr: fig("−14.20%"),
      real_mwr: fig("−15.00%"),
      real_provisional: true,
      benchmark: benchmark ? { asset_id: 7, name: "AFT", figure: fig("+6.40%"), real: fig("−17.00%") } : null,
      accounts: [],
      excluded: [],
      growth: [],
    },
  };
}
const incomeView = { ...EMPTY.cmd_income, totals: { gross: "45000.00", withheld: "6060.00", net: "38940.00" } };
const history = {
  base_currency: "TRY",
  valued_days: 3,
  points: [
    { date: "2025-12-31", value: "4410170.00", contributions: "4000000.00", estimated: false },
    { date: "2026-06-01", value: "4600000.00", contributions: "4200000.00", estimated: false },
    { date: "2026-11-01", value: "4812350.00", contributions: "4300000.00", estimated: false },
  ],
};

test("summary cards for YTD: net worth and change, returns with benchmark, income", async () => {
  serve({ ...overview, has_prices: true, total: t("4812350.00") }, { cmd_performance: perfView(true), cmd_income: incomeView, cmd_value_history: history });
  renderApp("/");
  const value = await screen.findByRole("region", { name: "Portfolio value" });
  expect(within(value).getByTestId("portfolio-total")).toHaveTextContent("4.81M TRY");
  expect(await within(value).findByTestId("value-change")).toHaveTextContent("+402.18K TRY this period");
  const twr = screen.getByRole("region", { name: "Time-weighted return" });
  expect(twr).toHaveTextContent("Time-weighted return · YTD");
  expect(await within(twr).findByText("+9.10%")).toBeInTheDocument();
  expect(within(twr).getByTestId("benchmark-line")).toHaveTextContent("Benchmark AFT: +6.40%");
  expect(within(screen.getByRole("region", { name: "Money-weighted return" })).getByText("+8.30%")).toBeInTheDocument();
  const inc = screen.getByRole("region", { name: "Income" });
  expect(await within(inc).findByTestId("income-net")).toHaveTextContent("38.94K TRY");
  expect(inc).toHaveTextContent("6.06K TRY tax withheld");
  expect(mockCall("cmd_performance")).toEqual({ period: "ytd", account_id: null });
});

test("no benchmark: no benchmark line", async () => {
  serve({ ...overview, has_prices: true }, { cmd_performance: perfView(false), cmd_income: incomeView, cmd_value_history: history });
  renderApp("/");
  const twr = await screen.findByRole("region", { name: "Time-weighted return" });
  expect(await within(twr).findByText("+9.10%")).toBeInTheDocument();
  expect(within(twr).queryByTestId("benchmark-line")).toBeNull();
});

test("Real shows real returns with the inflation month", async () => {
  serve(
    { ...overview, has_prices: true },
    { cmd_performance: perfView(true), cmd_income: incomeView, cmd_value_history: history, cmd_view_get: { account_id: null, period: "ytd", real: true } },
  );
  renderApp("/");
  const twr = await screen.findByRole("region", { name: "Time-weighted return" });
  expect(await within(twr).findByText("−14.20%")).toBeInTheDocument();
  expect(twr).toHaveTextContent("inflation through Sep 2026 · provisional");
  expect(within(screen.getByRole("region", { name: "Money-weighted return" })).getByText("−15.00%")).toBeInTheDocument();
});

function mockCall(cmd: string) {
  const calls = vi.mocked(invoke).mock.calls.filter((c) => c[0] === cmd);
  return calls[calls.length - 1]?.[1];
}

test("the exact amount is kept in the title of a short one", async () => {
  serve({ ...overview, has_prices: true, total: t("279949.48") });
  renderApp("/");
  const total = await screen.findByTestId("portfolio-total");
  expect(total).toHaveTextContent("279.95K TRY");
  expect(total).toHaveAttribute("title", "279,949.48 TRY");
});

test("the chart starts on the first day with a value", async () => {
  const early = [
    { date: "2014-04-20", value: null, contributions: "0.00", estimated: false },
    { date: "2014-04-21", value: "0.00", contributions: "0.00", estimated: false },
    ...points,
  ];
  serve(
    { ...overview, has_prices: true },
    { cmd_value_history: { base_currency: "TRY", points: early, valued_days: 3 }, cmd_view_get: { account_id: null, period: "since_start", real: false } },
  );
  renderApp("/");
  const card = await screen.findByRole("region", { name: "Value chart" });
  await waitFor(() => expect(within(card).getByRole("img")).toHaveAccessibleName(/30 Oct 2026 to 1 Nov 2026/));
});
