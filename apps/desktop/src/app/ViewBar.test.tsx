import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { EMPTY, invokeFrom, renderApp } from "../test/render";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
const mockInvoke = vi.mocked(invoke);
vi.mocked(listen);

const accounts = [
  { id: 1, name: "IBKR", hints: ["U1"] },
  { id: 2, name: "Pension", hints: ["P1"] },
];

function serve(extra: Record<string, unknown> = {}) {
  let stored = { account_id: null as number | null, period: "ytd", real: false };
  mockInvoke.mockImplementation(
    invokeFrom({
      ...EMPTY,
      cmd_accounts: accounts,
      cmd_view_get: () => stored,
      cmd_view_set: (a: unknown) => {
        stored = (a as { view: typeof stored }).view;
        return stored;
      },
      cmd_set_base_currency: undefined,
      ...extra,
    }) as typeof invoke,
  );
}

beforeEach(() => {
  mockInvoke.mockReset();
  serve();
});

const bar = () => screen.getByRole("toolbar", { name: "View" });

test("Performance shows scope, period, currency, Nominal | Real and Add", async () => {
  renderApp("/performance");
  const b = await waitFor(bar);
  expect(within(b).getByRole("combobox", { name: "Scope" })).toBeInTheDocument();
  expect(within(b).getByRole("group", { name: "Period" })).toBeInTheDocument();
  expect(await within(b).findByRole("combobox", { name: "Currency" })).toBeInTheDocument();
  expect(within(b).getByRole("group", { name: "Returns" })).toBeInTheDocument();
  expect(within(b).getByRole("button", { name: "Add" })).toBeInTheDocument();
});

test("Holdings shows scope, currency and Add, but no period or Nominal | Real", async () => {
  renderApp("/holdings");
  const b = await waitFor(bar);
  expect(within(b).getByRole("combobox", { name: "Scope" })).toBeInTheDocument();
  expect(await within(b).findByRole("combobox", { name: "Currency" })).toBeInTheDocument();
  expect(within(b).queryByRole("group", { name: "Period" })).toBeNull();
  expect(within(b).queryByRole("group", { name: "Returns" })).toBeNull();
  expect(within(b).getByRole("button", { name: "Add" })).toBeInTheDocument();
});

test("choosing a scope is saved and passed to the screen's query", async () => {
  renderApp("/holdings");
  const scope = await within(await waitFor(bar)).findByRole("combobox", { name: "Scope" });
  await waitFor(() => expect(within(scope).getByRole("option", { name: "Pension" })).toBeInTheDocument());
  await userEvent.selectOptions(scope, "Pension");
  await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("cmd_view_set", { view: { account_id: 2, period: "ytd", real: false } }));
  await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("cmd_holdings", { account_id: 2 }));
  expect(scope).toHaveValue("2");
});

test("the stored view is restored on start", async () => {
  serve({ cmd_view_get: { account_id: 1, period: "three_months", real: true } });
  renderApp("/performance");
  const b = await waitFor(bar);
  await waitFor(() => expect(within(b).getByRole("button", { name: "3M" })).toHaveAttribute("aria-pressed", "true"));
  expect(within(b).getByRole("button", { name: "Real" })).toHaveAttribute("aria-pressed", "true");
  await waitFor(() => expect(within(b).getByRole("combobox", { name: "Scope" })).toHaveValue("1"));
});

test("changing the base currency in the bar changes the setting", async () => {
  renderApp("/holdings");
  const ccy = await within(await waitFor(bar)).findByRole("combobox", { name: "Currency" });
  expect(ccy).toHaveValue("TRY");
  await userEvent.selectOptions(ccy, "EUR");
  await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("cmd_set_base_currency", { currency: "EUR" }));
});

test("Add menu: Import file… opens the Imports file picker", async () => {
  const { router } = renderApp("/holdings");
  await userEvent.click(within(await waitFor(bar)).getByRole("button", { name: "Add" }));
  const menu = screen.getByRole("menu", { name: "Add" });
  expect(within(menu).getAllByRole("menuitem").map((m) => m.textContent)).toEqual(["Import file…", "Add activity", "Add manual price"]);
  await userEvent.click(within(menu).getByRole("menuitem", { name: "Import file…" }));
  await waitFor(() => expect(router.state.location.pathname).toBe("/imports"));
  expect(router.state.location.state).toHaveProperty("pick");
});

test("Add menu closes on Escape", async () => {
  renderApp("/holdings");
  await userEvent.click(within(await waitFor(bar)).getByRole("button", { name: "Add" }));
  expect(screen.getByRole("menu")).toBeInTheDocument();
  await userEvent.keyboard("{Escape}");
  expect(screen.queryByRole("menu")).toBeNull();
});

test("Add activity opens the activity form with the scoped account chosen", async () => {
  serve({
    cmd_view_get: { account_id: 2, period: "ytd", real: false },
    cmd_activity_options: { ...EMPTY.cmd_activity_options, accounts: [{ id: 1, name: "IBKR" }, { id: 2, name: "Pension" }] },
    cmd_activity_check: { errors: [], warnings: [], duplicates: [] },
  });
  const { router } = renderApp("/holdings");
  await waitFor(() => expect(within(bar()).getByRole("combobox", { name: "Scope" })).toHaveValue("2"));
  await userEvent.click(within(bar()).getByRole("button", { name: "Add" }));
  await userEvent.click(screen.getByRole("menuitem", { name: "Add activity" }));
  await waitFor(() => expect(router.state.location.pathname).toBe("/activity"));
  const form = await screen.findByRole("region", { name: "Activity form" });
  expect(within(form).getByRole("combobox", { name: "Account of activity" })).toHaveValue("2");
});

test("Add manual price opens Assets with an asset picker", async () => {
  const asset = {
    asset_id: 1,
    instrument_key: "sym:AFT@",
    label: "AFT",
    name: null,
    class: "fund",
    currency: "TRY",
    source: { kind: "none", symbol: null },
    suggestion: null,
    latest_price: null,
    holders: 1,
    merged_into: null,
    manual_prices: [],
  };
  serve({ cmd_assets: [asset] });
  const { router } = renderApp("/holdings");
  await userEvent.click(within(await waitFor(bar)).getByRole("button", { name: "Add" }));
  await userEvent.click(screen.getByRole("menuitem", { name: "Add manual price" }));
  await waitFor(() => expect(router.state.location.pathname).toBe("/assets"));
  const card = await screen.findByRole("region", { name: "Add manual price" });
  const pick = within(card).getByRole("combobox");
  expect(pick).toHaveValue("");
  await userEvent.selectOptions(pick, "AFT");
  expect(within(card).getByLabelText("Price for AFT")).toBeInTheDocument();
  await userEvent.click(within(card).getByRole("button", { name: "Close" }));
  expect(screen.queryByRole("region", { name: "Add manual price" })).toBeNull();
});
