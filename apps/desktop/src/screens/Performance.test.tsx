import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Contributions } from "../bridge/generated/Contributions";
import type { Figure } from "../bridge/generated/Figure";
import type { PerformanceView } from "../bridge/generated/PerformanceView";
import type { Period } from "../bridge/generated/Period";
import { EMPTY, invokeFrom, renderApp } from "../test/render";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

const acct = (from: string, amount: string) => ({ account_id: 1, name: "Interactive Brokers", from, by_currency: [{ currency: "USD", amount, base_value: null }] });
const line = (amount: string) => [{ currency: "USD", amount, base_value: null }];
const contributions: Contributions = {
  periods: [
    { period: "one_month", to: "2026-11-01", accounts: [acct("2026-10-02", "+0.00")], total: line("+0.00") },
    { period: "three_months", to: "2026-11-01", accounts: [acct("2026-08-02", "+500.00")], total: line("+500.00") },
    { period: "ytd", to: "2026-11-01", accounts: [acct("2026-01-01", "+8500.00")], total: line("+8500.00") },
    { period: "last12_months", to: "2026-11-01", accounts: [acct("2025-11-02", "+8500.00")], total: line("+8500.00") },
    { period: "since_start", to: "2026-11-01", accounts: [acct("2025-03-01", "+10500.00")], total: line("+10500.00") },
  ],
};

const fig = (value: string | null, extra: Partial<Figure> = {}): Figure => ({ value, annualized: null, reason: null, estimated_days: 0, ...extra });

function view(period: Period, extra: Partial<PerformanceView["returns"]> = {}, top: Partial<PerformanceView> = {}): PerformanceView {
  return {
    base_currency: "TRY",
    benchmark_options: [{ asset_id: 7, name: "AFT · Ak Portföy Yeni Teknolojiler" }],
    benchmark_asset_id: null,
    inflation: { label: "HICP Türkiye (Eurostat)", through: "Sep 2026", unavailable: null },
    ...top,
    returns: {
      period,
      from: "2026-01-01",
      clamped: false,
      to: "2026-11-01",
      twr: fig("+12.34%"),
      mwr: fig("+11.00%"),
      real_twr: fig("−8.10%"),
      real_mwr: fig("−9.05%"),
      real_provisional: true,
      benchmark: null,
      accounts: [
        { account_id: 1, name: "Interactive Brokers", from: "2026-01-01", clamped: false, twr: fig("+12.34%"), mwr: fig("+11.00%"), real_twr: fig("−8.10%"), real_mwr: fig("−9.05%") },
        { account_id: 2, name: "Pension", from: "2026-08-14", clamped: true, twr: fig("−1.50%"), mwr: fig("−1.20%"), real_twr: fig("−6.40%"), real_mwr: fig("−6.10%") },
      ],
      excluded: [],
      growth: [
        { date: "2026-01-01", portfolio: "100.00", benchmark: null, portfolio_real: "100.00", benchmark_real: null },
        { date: "2026-11-01", portfolio: "112.34", benchmark: null, portfolio_real: "91.90", benchmark_real: null },
      ],
      ...extra,
    },
  };
}

function serve(perf: (period: Period) => PerformanceView = (p) => view(p), extra: Record<string, unknown> = {}) {
  vi.mocked(invoke).mockImplementation(
    invokeFrom({
      ...EMPTY,
      cmd_contributions: contributions,
      cmd_performance: (a: unknown) => perf((a as { period: Period }).period),
      cmd_set_benchmark: undefined,
      ...extra,
    }) as typeof invoke,
  );
}

beforeEach(() => {
  vi.mocked(invoke).mockReset();
  serve();
  vi.mocked(listen).mockImplementation((async () => () => {}) as unknown as typeof listen);
});

test("return cards, account rows and the growth chart", async () => {
  renderApp("/performance");
  const twr = await screen.findByRole("region", { name: "Time-weighted return" });
  expect(twr).toHaveTextContent("+12.34%");
  expect(screen.getByRole("region", { name: "Money-weighted return" })).toHaveTextContent("+11.00%");
  const rows = screen.getAllByTestId("return-row");
  expect(rows[0]).toHaveTextContent("1 Jan 2026 – 1 Nov 2026");
  expect(rows[1]).toHaveTextContent("since 14 Aug 2026");
  expect(rows[1]).toHaveTextContent("−1.50%");
  const growth = screen.getByRole("region", { name: "Growth chart" });
  expect(within(growth).getByRole("img")).toHaveAccessibleName(/Portfolio: 112\.34 at the end/);
  expect(screen.queryByTestId("estimate")).not.toBeInTheDocument();
});

test("periods include 1M and 3M and ask for that period", async () => {
  renderApp("/performance");
  await screen.findByRole("region", { name: "Time-weighted return" });
  await userEvent.click(screen.getByRole("button", { name: "1M" }));
  expect(invoke).toHaveBeenCalledWith("cmd_performance", { period: "one_month" });
  await userEvent.click(screen.getByRole("button", { name: "3M" }));
  expect(invoke).toHaveBeenCalledWith("cmd_performance", { period: "three_months" });
});

test("an account without exchange rates reads not available and the card says it is excluded", async () => {
  serve((p) =>
    view(p, {
      accounts: [
        { account_id: 1, name: "Interactive Brokers", from: "2026-01-01", clamped: false, twr: fig(null, { reason: "exchange rates before 2025-01-02 are missing" }), mwr: fig(null, { reason: "exchange rates before 2025-01-02 are missing" }), real_twr: fig(null, { reason: "exchange rates before 2025-01-02 are missing" }), real_mwr: fig(null, { reason: "exchange rates before 2025-01-02 are missing" }) },
      ],
      excluded: ["Interactive Brokers: exchange rates before 2025-01-02 are missing"],
    }),
  );
  renderApp("/performance");
  const row = await screen.findByTestId("return-row");
  expect(row).toHaveTextContent("not available: exchange rates before 2025-01-02 are missing");
  expect(screen.getByRole("region", { name: "Time-weighted return" })).toHaveTextContent("Excludes Interactive Brokers");
});

test("estimated figures carry the mark", async () => {
  serve((p) => view(p, { twr: fig("+4.00%", { estimated_days: 40 }) }));
  renderApp("/performance");
  const twr = await screen.findByRole("region", { name: "Time-weighted return" });
  expect(within(twr).getByTestId("estimate")).toHaveTextContent("estimate · 40 days at trade price or cost");
});

test("choosing a benchmark saves it and shows its return and line", async () => {
  let chosen: number | null = null;
  vi.mocked(invoke).mockImplementation(((cmd: string, args?: Record<string, unknown>) => {
    if (cmd === "cmd_set_benchmark") {
      chosen = args?.asset_id as number | null;
      return Promise.resolve(undefined);
    }
    if (cmd === "cmd_performance") return Promise.resolve(chosen === null ? view(args!.period as Period) : view(args!.period as Period, { benchmark: { asset_id: 7, name: "AFT", figure: fig("+18.00%"), real: fig("−2.00%") }, growth: [{ date: "2026-01-01", portfolio: "100.00", benchmark: "100.00", portfolio_real: "100.00", benchmark_real: "100.00" }, { date: "2026-11-01", portfolio: "112.34", benchmark: "118.00", portfolio_real: "91.90", benchmark_real: "96.50" }] }, { benchmark_asset_id: 7 }));
    return invokeFrom({ ...EMPTY, cmd_contributions: contributions })(cmd, args);
  }) as typeof invoke);
  renderApp("/performance");
  const card = await screen.findByRole("region", { name: "Benchmark" });
  expect(card).toHaveTextContent("Choose an asset with prices");
  await userEvent.selectOptions(within(card).getByRole("combobox", { name: "Benchmark" }), "7");
  expect(invoke).toHaveBeenCalledWith("cmd_set_benchmark", { asset_id: 7 });
  expect(await within(card).findByText("+18.00%")).toBeInTheDocument();
  const growth = screen.getByRole("region", { name: "Growth chart" });
  expect(within(growth).getByRole("list", { name: "Growth legend" })).toHaveTextContent("AFT");
});

test("net contributions per account and total, switching periods", async () => {
  renderApp("/performance");
  const row = await screen.findByTestId("contribution-row");
  expect(row).toHaveTextContent("+8500.00 USD");
  expect(row).toHaveTextContent("2026-01-01");
  expect(screen.getByTestId("contribution-total")).toHaveTextContent("+8500.00 USD");

  await userEvent.click(screen.getByRole("button", { name: "Since start" }));
  expect(screen.getByTestId("contribution-row")).toHaveTextContent("+10500.00 USD");
  expect(screen.getByTestId("contribution-row")).toHaveTextContent("2025-03-01");
  expect(screen.getByRole("button", { name: "Since start" })).toHaveAttribute("aria-pressed", "true");

  await userEvent.click(screen.getByRole("button", { name: "1Y" }));
  expect(screen.getByTestId("contribution-row")).toHaveTextContent("2025-11-02");
});

test("nominal by default; Real switches every figure and the chart, with the inflation note", async () => {
  serve((p) =>
    view(p, {
      real_twr: fig("−8.10%", { estimated_days: 40 }),
      twr: fig("+12.34%", { estimated_days: 40 }),
      benchmark: { asset_id: 7, name: "AFT", figure: fig("+18.00%"), real: fig("−2.00%") },
      growth: [
        { date: "2026-01-01", portfolio: "100.00", benchmark: "100.00", portfolio_real: "100.00", benchmark_real: "100.00" },
        { date: "2026-11-01", portfolio: "112.34", benchmark: "118.00", portfolio_real: "91.90", benchmark_real: "96.50" },
      ],
    }),
  );
  renderApp("/performance");
  const twr = await screen.findByRole("region", { name: "Time-weighted return" });
  expect(twr).toHaveTextContent("+12.34%");
  expect(screen.getByRole("button", { name: "Nominal" })).toHaveAttribute("aria-pressed", "true");
  expect(screen.queryByTestId("inflation-note")).not.toBeInTheDocument();

  await userEvent.click(screen.getByRole("button", { name: "Real" }));
  expect(screen.getByRole("region", { name: "Time-weighted return" })).toHaveTextContent("−8.10%");
  expect(within(screen.getByRole("region", { name: "Time-weighted return" })).getByTestId("estimate")).toHaveTextContent("estimate · 40 days");
  expect(screen.getByRole("region", { name: "Money-weighted return" })).toHaveTextContent("−9.05%");
  expect(screen.getByRole("region", { name: "Benchmark" })).toHaveTextContent("−2.00%");
  expect(screen.getAllByTestId("return-row")[1]).toHaveTextContent("−6.40%");
  expect(screen.getByTestId("inflation-note")).toHaveTextContent("HICP Türkiye (Eurostat) through Sep 2026 · provisional");
  const growth = screen.getByRole("region", { name: "Growth chart" });
  expect(within(growth).getByRole("img")).toHaveAccessibleName(/Portfolio: 91\.90 at the end/);
  expect(growth).toHaveTextContent("after inflation");
});

test("Real without an index shows the reason instead of figures", async () => {
  serve((p) => view(p, {}, { base_currency: "CHF", inflation: { label: null, through: null, unavailable: "no inflation index for CHF" } }));
  renderApp("/performance");
  await screen.findByRole("region", { name: "Time-weighted return" });
  await userEvent.click(screen.getByRole("button", { name: "Real" }));
  expect(screen.getByRole("region", { name: "Real returns" })).toHaveTextContent("Real returns are not available: no inflation index for CHF.");
  expect(screen.queryByRole("region", { name: "Time-weighted return" })).not.toBeInTheDocument();
});
