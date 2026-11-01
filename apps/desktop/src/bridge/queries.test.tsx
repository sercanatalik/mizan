import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { keys, useHoldings, useImportCommit, useImportDecide, useImportUndo, useResolveHint } from "./queries";

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
  expect(mockInvoke).toHaveBeenCalledWith("cmd_holdings");
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
