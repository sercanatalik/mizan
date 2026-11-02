import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { AccountDetail } from "../bridge/generated/AccountDetail";
import type { CurrentUnitsChart } from "../bridge/generated/CurrentUnitsChart";
import type { Figure } from "../bridge/generated/Figure";
import { EMPTY, invokeFrom, renderApp } from "../test/render";
import { foldBands } from "./Account";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
const mockInvoke = vi.mocked(invoke);

beforeEach(() => {
  vi.mocked(listen).mockResolvedValue(() => {});
});

const fig = (value: string | null, reason: string | null = null): Figure => ({ value, annualized: null, reason, estimated_days: 0 });

const detail = (extra: Partial<AccountDetail> = {}): AccountDetail => ({
  id: 1,
  name: "BES pension",
  identifiers: ["BES-A"],
  tax_treatment: "pension",
  price_source: null,
  delete_blocker: "it has 412 activities; merge it into another account instead",
  merge_targets: [{ id: 2, name: "IBKR" }],
  summary: {
    base_currency: "TRY",
    second_currency: "USD",
    value: "-98502.89",
    value_second: "-1978.48",
    rate_note: "USD/TRY 49.7872 on 2026-10-31",
    positions_value: "1200000.00",
    cash_value: "-1298502.89",
    cash: [{ amount: "-1298502.89", currency: "TRY" }],
    cost: null,
    unrealized: null,
    last_day: { amount: "+9098.00", pct: "+0.8%", date: "2026-10-31" },
    one_year: fig("+41.2%"),
    one_year_second: "+12.0%",
    ytd: fig("+30.1%"),
    notes: ["1 position left out: no price"],
  },
  holdings: [
    {
      key: "tefas:AET",
      symbol: "AET",
      name: "Altın katılım",
      units: "5458931",
      price: "0.180000",
      price_currency: "TRY",
      price_date: "2026-10-31",
      value: "982607.58",
      value_second: "19735.94",
      weight: "81.9%",
      cost: null,
      gain: null,
      day: "+0.5%",
      month: "−1.2%",
      year: null,
      note: null,
    },
    {
      key: "tefas:AH1",
      symbol: "AH1",
      name: null,
      units: "1000",
      price: "217.39",
      price_currency: "TRY",
      price_date: "2026-10-31",
      value: "217392.42",
      value_second: "4366.42",
      weight: "18.1%",
      cost: null,
      gain: null,
      day: null,
      month: null,
      year: "+22.0%",
      note: null,
    },
  ],
  total_value: "1200000.00",
  total_value_second: "24102.36",
  returns: [{ period: "ytd", from: "2026-01-01", twr: fig("+30.1%"), mwr: fig(null, "no cash flows") }],
  history: [
    { date: "2026-10-30", value: "-100000.00", contributions: "50000.00" },
    { date: "2026-10-31", value: "-98502.89", contributions: "50000.00" },
  ],
  income: [{ year: 2025, gross: "1200.00", withheld: "120.00" }],
  recent: [],
  checks: [{ as_of: "2026-10-31", passed: false, failed: ["AEA"] }],
  ...extra,
});

const chart: CurrentUnitsChart = {
  currency: "TRY",
  period: "one_year",
  dates: ["2026-10-30", "2026-10-31"],
  bands: [
    { key: "tefas:AET", name: "AET", values: ["980000", "982607.58"], short_history: false },
    { key: "tefas:AH1", name: "AH1", values: [null, "217392.42"], short_history: true },
  ],
  weights: ["81.9%", "18.1%"],
};

function serve(extra: Record<string, unknown> = {}) {
  mockInvoke.mockImplementation(
    invokeFrom({
      ...EMPTY,
      cmd_account_detail: detail(),
      cmd_account_chart: chart,
      cmd_account_rename: null,
      cmd_account_add_identifier: null,
      cmd_account_move_identifier: null,
      cmd_account_delete: null,
      cmd_account_merge_preview: { activities: 412, balance_checks: 3, identifiers: 1 },
      cmd_account_merge: 8,
      cmd_account_undo_merge: [],
      cmd_set_second_currency: null,
      cmd_account_set_price_source: { set: 7, skipped: [] },
      cmd_set_tax_treatment: null,
      ...extra,
    }) as typeof invoke,
  );
}

test("the account page shows tiles, holdings, chart and checks", async () => {
  serve();
  renderApp("/accounts/1");
  expect(await screen.findByRole("heading", { name: "BES pension", level: 1 })).toBeInTheDocument();
  const tiles = screen.getAllByTestId("tile");
  expect(tiles).toHaveLength(5);
  expect(tiles[0]).toHaveTextContent("−98,502.89");
  expect(tiles[1]).toHaveTextContent("USD/TRY 49.7872");
  expect(tiles[2]).toHaveTextContent("+9,098.00");
  expect(tiles[3]).toHaveTextContent("+41.2%");
  expect(tiles[3]).toHaveTextContent("+12.0% in USD");
  expect(tiles[4]).toHaveTextContent("1,200,000.00");
  expect(tiles[4]).toHaveTextContent("−1,298,502.89");
  const holdings = screen.getAllByTestId("holding-row");
  expect(holdings[0]).toHaveTextContent("81.9%");
  expect(holdings[0]).toHaveTextContent("−1.2%");
  expect(screen.getByTestId("positions-total-row")).toHaveTextContent("1,200,000.00");
  expect(screen.getByTestId("total-row")).toHaveTextContent("Account value");
  expect(screen.getByTestId("total-row")).toHaveTextContent("−98,502.89");
  expect(await screen.findByRole("figure", { name: "Value at current units" })).toBeInTheDocument();
  expect(screen.getByText(/shows price effect only/)).toBeInTheDocument();
  expect(screen.getByText(/Prices start later than the period for AH1/)).toBeInTheDocument();
  expect(screen.getByTestId("check-row")).toHaveTextContent("AEA");
  expect(screen.getByRole("link", { name: "All activity" })).toHaveAttribute("href", "/activity?account=1");
  expect(screen.getByTestId("delete-blocker")).toHaveTextContent("412 activities");
});

test("chart period and currency are sent to the backend", async () => {
  serve();
  renderApp("/accounts/1");
  await screen.findByRole("heading", { name: "BES pension", level: 1 });
  await userEvent.click(within(screen.getByRole("group", { name: "Chart period" })).getByRole("button", { name: "3Y" }));
  await userEvent.click(within(screen.getByRole("group", { name: "Chart currency" })).getByRole("button", { name: "USD" }));
  await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("cmd_account_chart", { id: 1, period: "three_years", currency: "USD" }));
});

test("rename, identifiers, tax treatment and second currency", async () => {
  serve();
  renderApp("/accounts/1");
  await screen.findByRole("heading", { name: "BES pension", level: 1 });
  await userEvent.click(screen.getByRole("button", { name: "Rename" }));
  const name = screen.getByRole("textbox", { name: "Account name" });
  await userEvent.clear(name);
  await userEvent.type(name, "Anadolu BES");
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("cmd_account_rename", { id: 1, name: "Anadolu BES" }));

  await userEvent.type(screen.getByRole("textbox", { name: "New identifier" }), "BES-B");
  await userEvent.click(screen.getByRole("button", { name: "Add" }));
  await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("cmd_account_add_identifier", { id: 1, identifier: "BES-B" }));

  await userEvent.selectOptions(screen.getByRole("combobox", { name: "Move BES-A to" }), "2");
  await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("cmd_account_move_identifier", { identifier: "BES-A", to: 2 }));

  await userEvent.selectOptions(screen.getByRole("combobox", { name: "Tax treatment" }), "isa");
  await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("cmd_set_tax_treatment", expect.objectContaining({ treatment: "isa" })));

  const second = screen.getByRole("combobox", { name: "Second currency" });
  expect(within(second).queryByRole("option", { name: "TRY" })).toBeNull();
  await userEvent.selectOptions(second, "EUR");
  await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("cmd_set_second_currency", { currency: "EUR" }));
});

test("choosing TEFAS for the account fills its assets' price sources", async () => {
  serve();
  renderApp("/accounts/1");
  await screen.findByRole("heading", { name: "BES pension", level: 1 });
  const select = screen.getByRole("combobox", { name: "Price source" });
  expect(select).toHaveValue("");
  await userEvent.selectOptions(select, "tefas");
  await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("cmd_account_set_price_source", { id: 1, provider: "tefas" }));
  expect(await screen.findByTestId("price-source-outcome")).toHaveTextContent("Price source set for 7 assets");
});

test("merge shows what will move before confirming", async () => {
  serve();
  renderApp("/accounts/1");
  await screen.findByRole("heading", { name: "BES pension", level: 1 });
  await userEvent.click(screen.getByRole("button", { name: "Merge…" }));
  const group = screen.getByRole("group", { name: "Merge account" });
  expect(within(group).getByRole("button", { name: "Merge" })).toBeDisabled();
  await userEvent.selectOptions(within(group).getByRole("combobox", { name: "Merge into" }), "2");
  expect(await within(group).findByTestId("merge-preview")).toHaveTextContent("412 activities, 3 balance checks and 1 identifiers will move to IBKR");
  expect(within(group).getByTestId("merge-preview")).toHaveTextContent("Transfers between the two accounts become internal");
  await userEvent.click(within(group).getByRole("button", { name: "Merge" }));
  await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("cmd_account_merge", { from: 1, into: 2 }));
});

test("an empty account can be deleted after confirming", async () => {
  serve({ cmd_account_detail: detail({ delete_blocker: null }) });
  renderApp("/accounts/1");
  await screen.findByRole("heading", { name: "BES pension", level: 1 });
  await userEvent.click(screen.getByRole("button", { name: "Delete…" }));
  expect(mockInvoke).not.toHaveBeenCalledWith("cmd_account_delete", expect.anything());
  await userEvent.click(screen.getByRole("button", { name: "Delete account" }));
  await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("cmd_account_delete", { id: 1 }));
  expect(await screen.findByRole("heading", { name: "Accounts", level: 1 })).toBeInTheDocument();
});

test("more than eight positions fold into Other, by weight", () => {
  const bands = Array.from({ length: 10 }, (_, i) => ({ name: `P${i}`, values: [i, null], weight: i }));
  const out = foldBands(bands);
  expect(out).toHaveLength(8);
  expect(out[0]!.name).toBe("P9");
  expect(out[7]).toEqual({ name: "Other (3)", values: [3, null] });
});
