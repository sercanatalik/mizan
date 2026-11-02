import { useMutation, useQuery, useQueryClient, type QueryClient } from "@tanstack/react-query";
import { api, type PriceSourceKind, type TaxTreatment } from "./api";
import type { Country } from "./generated/Country";
import type { Decision } from "./generated/Decision";
import type { ImportView } from "./generated/ImportView";
import type { Period } from "./generated/Period";

export const keys = {
  holdings: ["holdings"] as const,
  overview: ["overview"] as const,
  contributions: ["contributions"] as const,
  accounts: ["accounts"] as const,
  history: ["history"] as const,
  importSession: ["import"] as const,
  pending: ["pending"] as const,
  settings: ["settings"] as const,
  assets: ["assets"] as const,
  backfill: ["backfill"] as const,
  valueHistory: ["value-history"] as const,
  performance: ["performance"] as const,
  taxYears: ["tax-years"] as const,
  taxReport: ["tax-report"] as const,
  /** Client-side flag set by the background refresh events. */
  refreshing: ["refreshing"] as const,
};

export const useHoldings = () => useQuery({ queryKey: keys.holdings, queryFn: api.holdings });
export const useOverview = () => useQuery({ queryKey: keys.overview, queryFn: api.overview });
export const useContributions = () => useQuery({ queryKey: keys.contributions, queryFn: api.contributions });
export const useAccounts = () => useQuery({ queryKey: keys.accounts, queryFn: api.accounts });
export const useImportHistory = () => useQuery({ queryKey: keys.history, queryFn: api.importHistory });
export const useImportSession = () => useQuery({ queryKey: keys.importSession, queryFn: api.importCurrent });
export const usePendingDecisions = () => useQuery({ queryKey: keys.pending, queryFn: api.pendingDecisions });
export const useSettings = () => useQuery({ queryKey: keys.settings, queryFn: api.settings });
export const useAssets = () => useQuery({ queryKey: keys.assets, queryFn: api.assets });
export const useValueHistory = () => useQuery({ queryKey: keys.valueHistory, queryFn: api.valueHistory });
export const usePerformance = (period: Period) =>
  useQuery({ queryKey: [...keys.performance, period], queryFn: () => api.performance(period) });
export const useTaxYears = (country: Country) =>
  useQuery({ queryKey: [...keys.taxYears, country], queryFn: () => api.taxYears(country) });
export const useTaxReport = (country: Country, year: string | null) =>
  useQuery({
    queryKey: [...keys.taxReport, country, year],
    queryFn: () => api.taxReport(country, year as string),
    enabled: year !== null,
  });

/** Price history backfill; polled every 2 s while it runs. */
export const useBackfillStatus = () =>
  useQuery({
    queryKey: keys.backfill,
    queryFn: api.backfillStatus,
    refetchInterval: (q) => (q.state.data?.sources.some((s) => s.state === "running") ? 2000 : false),
  });

/** A review changed: show the new view and refresh what depends on it. */
function reviewChanged(qc: QueryClient, view: ImportView | null) {
  qc.setQueryData(keys.importSession, view);
  void qc.invalidateQueries({ queryKey: keys.pending });
  void qc.invalidateQueries({ queryKey: keys.overview });
}

/** Stored data changed: refresh every read model. */
function dataChanged(qc: QueryClient) {
  for (const k of [keys.holdings, keys.overview, keys.contributions, keys.accounts, keys.history, keys.pending, keys.valueHistory, keys.performance, keys.taxYears, keys.taxReport]) {
    void qc.invalidateQueries({ queryKey: k });
  }
}

/** Open a file. An unrecognized file leaves the current review as it is. */
export function useImportOpen() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: api.importOpen,
    onSuccess: (r) => {
      if (r.type === "review") reviewChanged(qc, r.view);
    },
  });
}

export function useImportChooseAdapter() {
  const qc = useQueryClient();
  return useMutation({ mutationFn: api.importChooseAdapter, onSuccess: (v) => reviewChanged(qc, v) });
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

/** Prices, rates or asset identity changed: refresh every view that shows values. */
export function valuesChanged(qc: QueryClient) {
  for (const k of [keys.holdings, keys.overview, keys.assets, keys.settings, keys.backfill, keys.valueHistory, keys.performance, keys.taxReport]) {
    void qc.invalidateQueries({ queryKey: k });
  }
}

/** A provider switch or key changed: Settings, the asset notes and backfill state depend on it. */
function providersChanged(qc: QueryClient) {
  for (const k of [keys.settings, keys.assets, keys.backfill]) {
    void qc.invalidateQueries({ queryKey: k });
  }
}

export function useRefreshPrices() {
  const qc = useQueryClient();
  return useMutation({ mutationFn: api.refreshPrices, onSettled: () => valuesChanged(qc) });
}

export function useSettingsMutations() {
  const qc = useQueryClient();
  return {
    setProviderEnabled: useMutation({
      mutationFn: (a: { id: string; enabled: boolean }) => api.setProviderEnabled(a.id, a.enabled),
      onSuccess: () => providersChanged(qc),
    }),
    setApiKey: useMutation({
      mutationFn: (a: { id: string; key: string | null }) => api.setApiKey(a.id, a.key),
      onSuccess: () => providersChanged(qc),
    }),
    setBaseCurrency: useMutation({ mutationFn: api.setBaseCurrency, onSuccess: () => valuesChanged(qc) }),
    setRefreshOnStart: useMutation({
      mutationFn: api.setRefreshOnStart,
      onSuccess: () => void qc.invalidateQueries({ queryKey: keys.settings }),
    }),
  };
}

export function useSetBenchmark() {
  const qc = useQueryClient();
  return useMutation({ mutationFn: api.setBenchmark, onSuccess: () => void qc.invalidateQueries({ queryKey: keys.performance }) });
}

export function useAssetMutations() {
  const qc = useQueryClient();
  const done = () => valuesChanged(qc);
  return {
    setPriceSource: useMutation({
      mutationFn: (a: { assetId: number; kind: PriceSourceKind; symbol: string | null }) => api.setPriceSource(a.assetId, a.kind, a.symbol),
      onSuccess: done,
    }),
    acceptSuggestion: useMutation({ mutationFn: api.acceptSuggestion, onSuccess: done }),
    addManualPrice: useMutation({
      mutationFn: (a: { assetId: number; date: string; value: string; currency: string }) => api.addManualPrice(a.assetId, a.date, a.value, a.currency),
      onSuccess: done,
    }),
    deleteManualPrice: useMutation({ mutationFn: (a: { assetId: number; date: string }) => api.deleteManualPrice(a.assetId, a.date), onSuccess: done }),
    mergeAssets: useMutation({ mutationFn: (a: { from: number; into: number }) => api.mergeAssets(a.from, a.into), onSuccess: done }),
    undoMerge: useMutation({ mutationFn: api.undoMerge, onSuccess: done }),
  };
}

export function useSetTaxTreatment() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (a: { accountId: number; treatment: TaxTreatment }) => api.setTaxTreatment(a.accountId, a.treatment),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: keys.taxYears });
      void qc.invalidateQueries({ queryKey: keys.taxReport });
    },
  });
}

export function useExportTaxCsv() {
  return useMutation({ mutationFn: (a: { country: Country; year: string; dir: string }) => api.exportTaxCsv(a.country, a.year, a.dir) });
}
