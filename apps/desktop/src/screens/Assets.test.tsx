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
  source: { kind: "none", fund_code: null },
  suggestion: null,
  latest_price: null,
  holders: 1,
  merged_into: null,
  manual_prices: [],
  ...a,
});

const assets: AssetView[] = [
  asset({ asset_id: 1, instrument_key: "sym:AFT@", label: "AFT", name: "Ak Portföy Yeni Teknolojiler", class: "fund", currency: "TRY", suggestion: "AFT" }),
  asset({
    asset_id: 2,
    instrument_key: "isin:IE00BK5BQT80",
    label: "VWCE",
    class: "etf",
    currency: "EUR",
    source: { kind: "manual", fund_code: null },
    latest_price: { value: "119.04", currency: "EUR", date: "2026-10-14", stale: true, source: "manual" },
    manual_prices: [{ date: "2026-10-14", value: "119.04", currency: "EUR" }],
  }),
  asset({ asset_id: 3, instrument_key: "sym:VWCE@XETR", label: "VWCE", holders: 0, merged_into: { merge_id: 7, asset_id: 2, label: "VWCE" } }),
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
  expect(await screen.findAllByTestId("asset-row")).toHaveLength(3);
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
  expect(aft).toHaveTextContent("Suggested: TEFAS AFT");
  await userEvent.click(within(aft).getByRole("button", { name: "Accept" }));
  expect(invoke).toHaveBeenCalledWith("cmd_accept_suggestion", { asset_id: 1 });
});

test("sets a TEFAS code, adds and deletes a manual price", async () => {
  renderApp("/assets");
  const aft = await row("sym:AFT@");
  await userEvent.click(within(aft).getByRole("button", { name: "Edit" }));
  const code = within(aft).getByLabelText("TEFAS fund code for AFT");
  await userEvent.type(code, "tcd");
  await userEvent.click(within(aft).getByRole("button", { name: "Use TEFAS" }));
  expect(invoke).toHaveBeenCalledWith("cmd_set_price_source", { asset_id: 1, kind: "tefas", fund_code: "TCD" });

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
