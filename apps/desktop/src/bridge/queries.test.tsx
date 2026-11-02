import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { keys, useAssetMutations, useHoldings, useImportCommit, useImportDecide, useImportUndo, useRefreshPrices, useResolveHint, useSettingsMutations } from "./queries";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const mockInvoke = vi.mocked(invoke);

function setup() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const wrapper = ({ children }: { children: ReactNode }) => <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
  return { qc, wrapper };
}

beforeEach(() => mockInvoke.mockReset());

test("query hooks call the matching command", async () => {
  mockInvoke.mockResolvedValue({ accounts: [] });
  const { wrapper } = setup();
  const { result } = renderHook(() => useHoldings(), { wrapper });
  await waitFor(() => expect(result.current.isSuccess).toBe(true));
  expect(mockInvoke).toHaveBeenCalledWith("cmd_holdings", { account_id: null });
});

test("decide sends snake_case args and stores the returned view", async () => {
  const view = { file_name: "b.json" };
  mockInvoke.mockResolvedValue(view);
  const { qc, wrapper } = setup();
  const { result } = renderHook(() => useImportDecide(), { wrapper });
  await act(() => result.current.mutateAsync({ index: 0, decision: "new" }));
  expect(mockInvoke).toHaveBeenCalledWith("cmd_import_decide", { index: 0, decision: "new" });
  expect(qc.getQueryData(keys.importSession)).toEqual(view);
});

test("mapping an account passes account_id and refreshes accounts", async () => {
  mockInvoke.mockResolvedValue({ file_name: "b.json" });
  const { qc, wrapper } = setup();
  const spy = vi.spyOn(qc, "invalidateQueries");
  const { result } = renderHook(() => useResolveHint(), { wrapper });
  await act(() => result.current.mapAccount.mutateAsync({ hint: "DEG-4417", accountId: 7 }));
  expect(mockInvoke).toHaveBeenCalledWith("cmd_import_map_account", { hint: "DEG-4417", account_id: 7 });
  expect(spy).toHaveBeenCalledWith({ queryKey: keys.accounts });
});

test("commit clears the review and invalidates every read model", async () => {
  mockInvoke.mockResolvedValue({ batch_id: 3, committed: 38, skipped_undecided: 2 });
  const { qc, wrapper } = setup();
  qc.setQueryData(keys.importSession, { file_name: "b.json" });
  const spy = vi.spyOn(qc, "invalidateQueries");
  const { result } = renderHook(() => useImportCommit(), { wrapper });
  await act(() => result.current.mutateAsync());
  expect(qc.getQueryData(keys.importSession)).toBeNull();
  for (const k of [keys.holdings, keys.overview, keys.contributions, keys.history, keys.pending]) {
    expect(spy).toHaveBeenCalledWith({ queryKey: k });
  }
});

test("undo passes batch_id", async () => {
  mockInvoke.mockResolvedValue(undefined);
  const { wrapper } = setup();
  const { result } = renderHook(() => useImportUndo(), { wrapper });
  await act(() => result.current.mutateAsync(12));
  expect(mockInvoke).toHaveBeenCalledWith("cmd_import_undo", { batch_id: 12 });
});

test("market wrappers send snake_case args and refresh values", async () => {
  mockInvoke.mockResolvedValue(undefined);
  const { qc, wrapper } = setup();
  const spy = vi.spyOn(qc, "invalidateQueries");
  const { result } = renderHook(() => useAssetMutations(), { wrapper });
  await act(() => result.current.setPriceSource.mutateAsync({ assetId: 3, kind: "tefas", symbol: "AFT" }));
  expect(mockInvoke).toHaveBeenCalledWith("cmd_set_price_source", { asset_id: 3, kind: "tefas", symbol: "AFT" });
  await act(() => result.current.setPriceSource.mutateAsync({ assetId: 3, kind: "yahoo", symbol: "VWCE.DE" }));
  expect(mockInvoke).toHaveBeenCalledWith("cmd_set_price_source", { asset_id: 3, kind: "yahoo", symbol: "VWCE.DE" });
  await act(() => result.current.addManualPrice.mutateAsync({ assetId: 3, date: "2026-10-30", value: "1.25", currency: "TRY" }));
  expect(mockInvoke).toHaveBeenCalledWith("cmd_add_manual_price", { asset_id: 3, date: "2026-10-30", value: "1.25", currency: "TRY" });
  await act(() => result.current.mergeAssets.mutateAsync({ from: 4, into: 3 }));
  expect(mockInvoke).toHaveBeenCalledWith("cmd_merge_assets", { from: 4, into: 3 });
  for (const k of [keys.holdings, keys.overview, keys.assets]) expect(spy).toHaveBeenCalledWith({ queryKey: k });
});

test("refresh calls the command and refreshes every value view", async () => {
  mockInvoke.mockResolvedValue({ started_at: "a", finished_at: "b", sources: [] });
  const { qc, wrapper } = setup();
  const spy = vi.spyOn(qc, "invalidateQueries");
  const { result } = renderHook(() => useRefreshPrices(), { wrapper });
  await act(() => result.current.mutateAsync());
  expect(mockInvoke).toHaveBeenCalledWith("cmd_refresh_prices");
  for (const k of [keys.holdings, keys.overview, keys.assets, keys.settings]) expect(spy).toHaveBeenCalledWith({ queryKey: k });
});

test("settings mutations", async () => {
  mockInvoke.mockResolvedValue(undefined);
  const { wrapper } = setup();
  const { result } = renderHook(() => useSettingsMutations(), { wrapper });
  await act(() => result.current.setProviderEnabled.mutateAsync({ id: "tefas", enabled: false }));
  expect(mockInvoke).toHaveBeenCalledWith("cmd_set_provider_enabled", { id: "tefas", enabled: false });
  await act(() => result.current.setApiKey.mutateAsync({ id: "twelvedata", key: "abc123" }));
  expect(mockInvoke).toHaveBeenCalledWith("cmd_set_api_key", { id: "twelvedata", key: "abc123" });
  await act(() => result.current.setApiKey.mutateAsync({ id: "twelvedata", key: null }));
  expect(mockInvoke).toHaveBeenCalledWith("cmd_set_api_key", { id: "twelvedata", key: null });
  await act(() => result.current.setBaseCurrency.mutateAsync("EUR"));
  expect(mockInvoke).toHaveBeenCalledWith("cmd_set_base_currency", { currency: "EUR" });
});
