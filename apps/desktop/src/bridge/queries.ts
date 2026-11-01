import { useMutation, useQuery, useQueryClient, type QueryClient } from "@tanstack/react-query";
import { api } from "./api";
import type { Decision } from "./generated/Decision";
import type { ImportView } from "./generated/ImportView";

export const keys = {
  holdings: ["holdings"] as const,
  overview: ["overview"] as const,
  contributions: ["contributions"] as const,
  accounts: ["accounts"] as const,
  history: ["history"] as const,
  importSession: ["import"] as const,
  pending: ["pending"] as const,
};

export const useHoldings = () => useQuery({ queryKey: keys.holdings, queryFn: api.holdings });
export const useOverview = () => useQuery({ queryKey: keys.overview, queryFn: api.overview });
export const useContributions = () => useQuery({ queryKey: keys.contributions, queryFn: api.contributions });
export const useAccounts = () => useQuery({ queryKey: keys.accounts, queryFn: api.accounts });
export const useImportHistory = () => useQuery({ queryKey: keys.history, queryFn: api.importHistory });
export const useImportSession = () => useQuery({ queryKey: keys.importSession, queryFn: api.importCurrent });
export const usePendingDecisions = () => useQuery({ queryKey: keys.pending, queryFn: api.pendingDecisions });

/** A review changed: show the new view and refresh what depends on it. */
function reviewChanged(qc: QueryClient, view: ImportView | null) {
  qc.setQueryData(keys.importSession, view);
  void qc.invalidateQueries({ queryKey: keys.pending });
  void qc.invalidateQueries({ queryKey: keys.overview });
}

/** Stored data changed: refresh every read model. */
function dataChanged(qc: QueryClient) {
  for (const k of [keys.holdings, keys.overview, keys.contributions, keys.accounts, keys.history, keys.pending]) {
    void qc.invalidateQueries({ queryKey: k });
  }
}

export function useImportOpen() {
  const qc = useQueryClient();
  return useMutation({ mutationFn: api.importOpen, onSuccess: (v) => reviewChanged(qc, v) });
}

export function useImportDecide() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (a: { index: number; decision: Decision | null }) => api.importDecide(a.index, a.decision),
    onSuccess: (v) => reviewChanged(qc, v),
  });
}

/** Resolving a hint writes a mapping, so accounts change too. */
export function useResolveHint() {
  const qc = useQueryClient();
  const done = (v: ImportView) => {
    reviewChanged(qc, v);
    void qc.invalidateQueries({ queryKey: keys.accounts });
  };
  return {
    createAccount: useMutation({ mutationFn: api.importCreateAccount, onSuccess: done }),
    mapAccount: useMutation({
      mutationFn: (a: { hint: string; accountId: number }) => api.importMapAccount(a.hint, a.accountId),
      onSuccess: done,
    }),
    createAsset: useMutation({ mutationFn: api.importCreateAsset, onSuccess: done }),
  };
}

export function useImportCommit() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: api.importCommit,
    onSuccess: () => {
      qc.setQueryData(keys.importSession, null);
      dataChanged(qc);
    },
  });
}

export function useImportClose() {
  const qc = useQueryClient();
  return useMutation({ mutationFn: api.importClose, onSuccess: () => reviewChanged(qc, null) });
}

export function useImportUndo() {
  const qc = useQueryClient();
  return useMutation({ mutationFn: api.importUndo, onSuccess: () => dataChanged(qc) });
}
