import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import type { IncomeView } from "../bridge/generated/IncomeView";
import { EMPTY, invokeFrom, renderApp } from "../test/render";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
const mockInvoke = vi.mocked(invoke);
vi.mocked(listen);

const income: IncomeView = {
  base_currency: "TRY",
  from: "2026-01-01",
  to: "2026-11-02",
  totals: { gross: "2000.00", withheld: "300.00", net: "1700.00" },
  previous: { gross: "3400.00", withheld: "510.00", net: "2890.00" },
  previous_from: "2025-01-01",
  previous_to: "2025-11-02",
  not_converted: 1,
  months: ["01", "02", "03", "04", "05", "06", "07", "08", "09", "10", "11"].map((m) => ({
    month: `2026-${m}`,
    dividends: m === "03" ? "1700.00" : "0.00",
    interest: "0.00",
  })),
  instruments: [{ instrument_key: "isin:US0378331005", label: "AAPL", account_id: 1, account: "IBKR", payments: 1, gross: "2000.00", withheld: "300.00", net: "1700.00" }],
  payments: [
    { date: "2026-03-03", account_id: 1, account: "IBKR", instrument_key: "isin:US0378331005", label: "AAPL", kind: "dividend", currency: "USD", gross: "50.00", withheld: "7.50", net: "42.50", base_gross: "2000.00", base_withheld: "300.00", base_net: "1700.00", note: null },
    { date: "2026-05-04", account_id: 1, account: "IBKR", instrument_key: null, label: "Interest · EUR", kind: "interest", currency: "EUR", gross: "3.00", withheld: "0.00", net: "3.00", base_gross: null, base_withheld: null, base_net: null, note: "not converted: no rate" },
  ],
};

function serve(v: IncomeView, extra: Record<string, unknown> = {}) {
  mockInvoke.mockImplementation(invokeFrom({ ...EMPTY, cmd_income: v, ...extra }) as typeof invoke);
}

beforeEach(() => {
  mockInvoke.mockReset();
  serve(income);
});

test("summary with the previous period, and the payments left out", async () => {
  renderApp("/income");
  const net = await screen.findByRole("region", { name: "Net income" });
  expect(net).toHaveTextContent("1,700.00 TRY");
  expect(net).toHaveTextContent("Previous period: 2,890.00");
  expect(screen.getByRole("region", { name: "Tax withheld" })).toHaveTextContent("300.00 TRY");
  expect(screen.getByRole("note")).toHaveTextContent("Totals leave out 1 payment that could not be converted to TRY");
  const payments = within(screen.getByRole("region", { name: "Payments" })).getAllByTestId("payment-row");
  expect(payments[0]).toHaveTextContent("not converted: no rate");
  expect(mockInvoke).toHaveBeenCalledWith("cmd_income", { period: "ytd", account_id: null });
});

test("months without income are shown empty", async () => {
  renderApp("/income");
  const chart = within(await screen.findByRole("region", { name: "Income by month" })).getByRole("img");
  expect(chart).toHaveAccessibleName(/Jan 2026: none/);
  expect(chart).toHaveAccessibleName(/Mar 2026: 1700\.00 TRY/);
});

test("an instrument row opens its payments in Activity", async () => {
  const { router } = renderApp("/income");
  await userEvent.click(await screen.findByRole("button", { name: "Show AAPL payments in IBKR" }));
  await waitFor(() => expect(router.state.location.pathname).toBe("/activity"));
  const q = new URLSearchParams(router.state.location.search);
  expect(Object.fromEntries(q)).toEqual({ account: "1", text: "AAPL", kinds: "dividend,distribution", from: "2026-01-01", to: "2026-11-02" });
});

test("no income names the period", async () => {
  serve({ ...income, payments: [], instruments: [], months: [], not_converted: 0 }, { cmd_view_get: { account_id: null, period: "one_month", real: false } });
  renderApp("/income");
  const status = await screen.findByRole("status");
  expect(status).toHaveTextContent("No income in the last month");
  expect(status.textContent).not.toMatch(/\d/);
});

test("export writes the CSV into the chosen folder", async () => {
  vi.mocked(open).mockResolvedValue("/Users/a/Documents");
  serve(income, { cmd_export_income_csv: "/Users/a/Documents/mizan-income-all-accounts-2026-01-01-2026-11-02.csv" });
  renderApp("/income");
  await userEvent.click(await screen.findByRole("button", { name: "Export CSV" }));
  expect(await screen.findByTestId("export-done")).toHaveTextContent("Saved mizan-income-all-accounts-2026-01-01-2026-11-02.csv");
  expect(mockInvoke).toHaveBeenCalledWith("cmd_export_income_csv", { period: "ytd", account_id: null, dir: "/Users/a/Documents" });
});
