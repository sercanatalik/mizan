import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { AssetView } from "../bridge/generated/AssetView";
import { EMPTY, invokeFrom, renderApp } from "../test/render";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

const asset = (a: Partial<AssetView> & Pick<AssetView, "asset_id" | "instrument_key" | "label">): AssetView => ({
  name: null,
  class: "other",
  currency: null,
  source: { kind: "none", symbol: null },
  suggestion: null,
  latest_price: null,
  holders: 1,
  merged_into: null,
  manual_prices: [],
  ...a,
});

const assets: AssetView[] = [
  asset({ asset_id: 1, instrument_key: "sym:AFT@", label: "AFT", name: "Ak Portföy Yeni Teknolojiler", class: "fund", currency: "TRY", suggestion: { kind: "tefas", symbol: "AFT", label: "TEFAS: AFT" } }),
  asset({
    asset_id: 2,
    instrument_key: "isin:IE00BK5BQT80",
    label: "VWCE",
    class: "etf",
    currency: "EUR",
    source: { kind: "manual", symbol: null },
    latest_price: { value: "119.04", currency: "EUR", date: "2026-10-14", stale: true, source: "manual" },
    manual_prices: [{ date: "2026-10-14", value: "119.04", currency: "EUR" }],
  }),
  asset({ asset_id: 3, instrument_key: "sym:VWCE@XETR", label: "VWCE", holders: 0, merged_into: { merge_id: 7, asset_id: 2, label: "VWCE" } }),
  asset({
    asset_id: 4,
    instrument_key: "sym:AAPL@XNAS",
    label: "AAPL",
    name: "Apple",
    class: "equity",
    currency: "USD",
    suggestion: { kind: "yahoo", symbol: "AAPL", label: "Yahoo: AAPL" },
  }),
  asset({
    asset_id: 5,
    instrument_key: "sym:MSFT@XNAS",
    label: "MSFT",
    class: "equity",
    currency: "USD",
    source: { kind: "twelvedata", symbol: "MSFT" },
  }),
  asset({
    asset_id: 6,
    instrument_key: "sym:VUSA@XLON",
    label: "VUSA",
    class: "etf",
    currency: "GBP",
    source: { kind: "yahoo", symbol: "VUSA.L" },
    latest_price: { value: "109.4", currency: "GBP", date: "2026-10-30", stale: false, source: "yahoo" },
  }),
];

beforeEach(() => {
  vi.mocked(invoke).mockReset();
  vi.mocked(listen).mockImplementation((async () => () => {}) as unknown as typeof listen);
  vi.mocked(invoke).mockImplementation(
    invokeFrom({
      ...EMPTY,
      cmd_assets: assets,
      cmd_accept_suggestion: undefined,
      cmd_set_price_source: undefined,
      cmd_add_manual_price: undefined,
      cmd_delete_manual_price: undefined,
      cmd_merge_assets: 9,
      cmd_undo_merge: undefined,
    }) as typeof invoke,
  );
});

const row = async (key: string) => (await screen.findByText(key)).closest("tr")!;

test("lists assets with source, latest price and holders", async () => {
  renderApp("/assets");
  expect(await screen.findAllByTestId("asset-row")).toHaveLength(6);
  const vwce = await row("isin:IE00BK5BQT80");
  expect(vwce).toHaveTextContent("Manual");
  expect(vwce).toHaveTextContent("119.04 EUR");
  expect(vwce).toHaveTextContent("stale · 14 Oct 2026");
  expect(await row("sym:AFT@")).toHaveTextContent("no price");
  expect(await row("sym:VWCE@XETR")).toHaveTextContent("Merged into VWCE");
});

test("accepts the TEFAS suggestion", async () => {
  renderApp("/assets");
  const aft = await row("sym:AFT@");
  expect(aft).toHaveTextContent("Suggested: TEFAS: AFT");
  await userEvent.click(within(aft).getByRole("button", { name: "Accept" }));
  expect(invoke).toHaveBeenCalledWith("cmd_accept_suggestion", { asset_id: 1 });
});

test("sets a TEFAS code, adds and deletes a manual price", async () => {
  renderApp("/assets");
  const aft = await row("sym:AFT@");
  await userEvent.click(within(aft).getByRole("button", { name: "Edit" }));
  await userEvent.selectOptions(within(aft).getByLabelText("Price source for AFT"), "tefas");
  const code = within(aft).getByLabelText("TEFAS fund code for AFT");
  await userEvent.type(code, "tcd");
  await userEvent.click(within(aft).getByRole("button", { name: "Save source" }));
  expect(invoke).toHaveBeenCalledWith("cmd_set_price_source", { asset_id: 1, kind: "tefas", symbol: "TCD" });

  const date = within(aft).getByLabelText("Price date for AFT");
  await userEvent.clear(date);
  await userEvent.type(date, "2026-10-30");
  await userEvent.type(within(aft).getByLabelText("Price for AFT"), "1.25");
  await userEvent.click(within(aft).getByRole("button", { name: "Add price" }));
  expect(invoke).toHaveBeenCalledWith("cmd_add_manual_price", { asset_id: 1, date: "2026-10-30", value: "1.25", currency: "TRY" });

  const vwce = await row("isin:IE00BK5BQT80");
  await userEvent.click(within(vwce).getByRole("button", { name: "Edit" }));
  await userEvent.click(within(vwce).getByRole("button", { name: "Delete" }));
  expect(invoke).toHaveBeenCalledWith("cmd_delete_manual_price", { asset_id: 2, date: "2026-10-14" });
});

test("merges an asset and undoes a merge", async () => {
  renderApp("/assets");
  const aft = await row("sym:AFT@");
  await userEvent.click(within(aft).getByRole("button", { name: "Edit" }));
  await userEvent.selectOptions(within(aft).getByLabelText("Merge AFT into"), "2");
  await userEvent.click(within(aft).getByRole("button", { name: "Merge" }));
  expect(invoke).toHaveBeenCalledWith("cmd_merge_assets", { from: 1, into: 2 });

  const merged = await row("sym:VWCE@XETR");
  await userEvent.click(within(merged).getByRole("button", { name: "Edit" }));
  await userEvent.click(within(merged).getByRole("button", { name: "Undo merge" }));
  expect(invoke).toHaveBeenCalledWith("cmd_undo_merge", { merge_id: 7 });
});

test("chooses Yahoo Finance in the asset menu", async () => {
  renderApp("/assets");
  const apple = await row("sym:AAPL@XNAS");
  await userEvent.click(within(apple).getByRole("button", { name: "Edit" }));
  await userEvent.selectOptions(within(apple).getByLabelText("Price source for AAPL"), "yahoo");
  await userEvent.type(within(apple).getByLabelText("Symbol for AAPL"), "aapl");
  await userEvent.click(within(apple).getByRole("button", { name: "Save source" }));
  expect(invoke).toHaveBeenCalledWith("cmd_set_price_source", { asset_id: 4, kind: "yahoo", symbol: "AAPL" });
});

test("a provider source needs a symbol", async () => {
  renderApp("/assets");
  const apple = await row("sym:AAPL@XNAS");
  await userEvent.click(within(apple).getByRole("button", { name: "Edit" }));
  await userEvent.selectOptions(within(apple).getByLabelText("Price source for AAPL"), "twelvedata");
  await userEvent.click(within(apple).getByRole("button", { name: "Save source" }));
  expect(within(apple).getByRole("alert")).toHaveTextContent("A symbol is required");
  expect(invoke).not.toHaveBeenCalledWith("cmd_set_price_source", expect.anything());

  // Manual needs no symbol.
  await userEvent.selectOptions(within(apple).getByLabelText("Price source for AAPL"), "manual");
  expect(within(apple).queryByLabelText("Symbol for AAPL")).not.toBeInTheDocument();
  await userEvent.click(within(apple).getByRole("button", { name: "Save source" }));
  expect(invoke).toHaveBeenCalledWith("cmd_set_price_source", { asset_id: 4, kind: "manual", symbol: null });
});

test("listed sources show their provider and why they will not refresh", async () => {
  vi.mocked(invoke).mockImplementation(
    invokeFrom({
      ...EMPTY,
      cmd_assets: assets,
      cmd_settings: {
        ...EMPTY.cmd_settings,
        providers: EMPTY.cmd_settings.providers.map((p) => (p.id === "yahoo" ? { ...p, enabled: false } : p)),
      },
    }) as typeof invoke,
  );
  renderApp("/assets");
  const vusa = await row("sym:VUSA@XLON");
  expect(vusa).toHaveTextContent("Yahoo · VUSA.L");
  expect(vusa).toHaveTextContent("109.4 GBP");
  expect(await within(vusa).findByTestId("source-note")).toHaveTextContent("provider off in Settings");
  const msft = await row("sym:MSFT@XNAS");
  expect(msft).toHaveTextContent("Twelve Data · MSFT");
  expect(within(msft).getByTestId("source-note")).toHaveTextContent("needs an API key");
});

test("accepts a Yahoo suggestion", async () => {
  renderApp("/assets");
  const apple = await row("sym:AAPL@XNAS");
  expect(apple).toHaveTextContent("Suggested: Yahoo: AAPL");
  await userEvent.click(within(apple).getByRole("button", { name: "Accept" }));
  expect(invoke).toHaveBeenCalledWith("cmd_accept_suggestion", { asset_id: 4 });
});
