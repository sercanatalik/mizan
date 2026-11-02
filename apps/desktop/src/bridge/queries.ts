import { useMutation, useQuery, useQueryClient, type QueryClient } from "@tanstack/react-query";
import { api, type PriceSourceKind, type ResetScopeName, type SeriesKindName, type TaxTreatment } from "./api";
import type { SeriesFilter } from "./generated/SeriesFilter";
import type { ChartPeriod } from "./generated/ChartPeriod";
import type { Country } from "./generated/Country";
import type { ActivityForm } from "./generated/ActivityForm";
import type { ActivityQuery } from "./generated/ActivityQuery";
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
  activities: ["activities"] as const,
  marketSeries: ["market-series"] as const,
  accountsList: ["accounts-list"] as const,
  accountDetail: ["account-detail"] as const,
  accountChart: ["account-chart"] as const,
  marketDetail: ["market-detail"] as const,
  activityDetail: ["activity-detail"] as const,
  activityOptions: ["activity-options"] as const,
  taxReport: ["tax-report"] as const,
  view: ["view"] as const,
  adapters: ["adapters"] as const,
  income: ["income"] as const,
  /** The Inbox and its count; both are derived from everything else. */
  inbox: ["inbox"] as const,
  /** Client-side flag set by the background refresh events. */
  refreshing: ["refreshing"] as const,
};

/** `accountId`: the account in scope, or null for all accounts. */
export const useHoldings = (accountId: number | null = null) =>
  useQuery({ queryKey: [...keys.holdings, accountId], queryFn: () => api.holdings(accountId) });
export const useOverview = (accountId: number | null = null) =>
  useQuery({ queryKey: [...keys.overview, accountId], queryFn: () => api.overview(accountId) });
export const useContributions = (accountId: number | null = null) =>
  useQuery({ queryKey: [...keys.contributions, accountId], queryFn: () => api.contributions(accountId) });
export const useAccounts = () => useQuery({ queryKey: keys.accounts, queryFn: api.accounts });
export const useImportHistory = () => useQuery({ queryKey: keys.history, queryFn: api.importHistory });
export const useImportSession = () => useQuery({ queryKey: keys.importSession, queryFn: api.importCurrent });
export const usePendingDecisions = () => useQuery({ queryKey: keys.pending, queryFn: api.pendingDecisions });
export const useIncome = (period: Period, accountId: number | null) =>
  useQuery({ queryKey: [...keys.income, period, accountId], queryFn: () => api.income(period, accountId) });
export const useAdapters = () => useQuery({ queryKey: keys.adapters, queryFn: api.adaptersList });

export function useAdapterMutations() {
  const qc = useQueryClient();
  const set = (l: Awaited<ReturnType<typeof api.adaptersList>>) => qc.setQueryData(keys.adapters, l);
  return {
    rename: useMutation({ mutationFn: (a: { id: string; name: string }) => api.mappingRename(a.id, a.name), onSuccess: set }),
    remove: useMutation({ mutationFn: api.mappingDelete, onSuccess: set }),
    exportTo: useMutation({ mutationFn: (a: { id: string; dest: string }) => api.adapterExport(a.id, a.dest) }),
  };
}

export const useInbox = () => useQuery({ queryKey: [...keys.inbox, "items"], queryFn: api.inbox });
export const useInboxCount = () => useQuery({ queryKey: [...keys.inbox, "count"], queryFn: api.inboxCount });

export function useInboxMutations() {
  const qc = useQueryClient();
  const done = () => void qc.invalidateQueries({ queryKey: keys.inbox });
  return {
    snooze: useMutation({ mutationFn: (a: { key: string; days: 1 | 7 }) => api.inboxSnooze(a.key, a.days), onSuccess: done }),
    unsnooze: useMutation({ mutationFn: api.inboxUnsnooze, onSuccess: done }),
  };
}
export const useSettings = () => useQuery({ queryKey: keys.settings, queryFn: api.settings });
export const useAssets = () => useQuery({ queryKey: keys.assets, queryFn: api.assets });
export const useValueHistory = (accountId: number | null = null) =>
  useQuery({ queryKey: [...keys.valueHistory, accountId], queryFn: () => api.valueHistory(accountId) });
export const usePerformance = (period: Period, accountId: number | null = null) =>
  useQuery({ queryKey: [...keys.performance, period, accountId], queryFn: () => api.performance(period, accountId) });
/** Pages of activities for a query; `pages` pages of 200 are loaded and joined. */
export const useActivities = (query: ActivityQuery, pages: number) =>
  useQuery({
    queryKey: [...keys.activities, query, pages],
    queryFn: async () => {
      const first = await api.activities(query, 0);
      const rest = [];
      for (let i = 1; i < pages && i * 200 < first.total; i++) rest.push(await api.activities(query, i * 200));
      return { ...first, rows: [first, ...rest].flatMap((p) => p.rows) };
    },
    placeholderData: (prev) => prev,
  });
export const useActivityDetail = (id: number | null) =>
  useQuery({ queryKey: [...keys.activityDetail, id], queryFn: () => api.activityDetail(id as number), enabled: id !== null });
export const useActivityOptions = () => useQuery({ queryKey: keys.activityOptions, queryFn: api.activityOptions });
export const useMarketSeries = (kind: SeriesKindName, filter: SeriesFilter) =>
  useQuery({ queryKey: [...keys.marketSeries, kind, filter], queryFn: () => api.marketSeries(kind, filter), placeholderData: (prev) => prev });

/** One series: `pages` pages of 200 values joined, newest first. */
export const useMarketDetail = (kind: SeriesKindName, key: string | null, pages: number) =>
  useQuery({
    queryKey: [...keys.marketDetail, kind, key, pages],
    enabled: key !== null,
    queryFn: async () => {
      const first = await api.marketSeriesDetail(kind, key as string, 0);
      const rest = [];
      for (let i = 1; i < pages && i * 200 < first.total; i++) rest.push(await api.marketSeriesDetail(kind, key as string, i * 200));
      return { ...first, points: [first, ...rest].flatMap((p) => p.points) };
    },
    placeholderData: (prev) => prev,
  });
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
  void qc.invalidateQueries({ queryKey: keys.inbox });
  void qc.invalidateQueries({ queryKey: keys.overview });
}

/** Stored data changed: refresh every read model. */
function dataChanged(qc: QueryClient) {
  for (const k of [keys.view, keys.inbox, keys.income, keys.holdings, keys.overview, keys.contributions, keys.accounts, keys.history, keys.pending, keys.valueHistory, keys.performance, keys.taxYears, keys.taxReport, keys.activities, keys.activityDetail, keys.activityOptions, keys.accountsList, keys.accountDetail, keys.accountChart]) {
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
  for (const k of [keys.inbox, keys.income, keys.holdings, keys.overview, keys.assets, keys.settings, keys.backfill, keys.valueHistory, keys.performance, keys.taxReport, keys.marketSeries, keys.marketDetail, keys.accountsList, keys.accountDetail, keys.accountChart]) {
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
    setBenchmarkCandidate: useMutation({ mutationFn: (a: { assetId: number; on: boolean }) => api.setBenchmarkCandidate(a.assetId, a.on), onSuccess: done }),
    addBenchmarkInstrument: useMutation({
      mutationFn: (a: { kind: "tefas" | "yahoo" | "twelvedata"; symbol: string; name: string | null }) => api.addBenchmarkInstrument(a.kind, a.symbol, a.name),
      onSuccess: done,
    }),
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

/** Manual entries: every save or delete changes stored data. */
export function useActivityMutations() {
  const qc = useQueryClient();
  const done = () => {
    dataChanged(qc);
    void qc.invalidateQueries({ queryKey: keys.assets });
  };
  return {
    save: useMutation({
      mutationFn: (a: { form: ActivityForm; confirmDuplicate: boolean; replaceId: number | null }) =>
        api.activitySave(a.form, a.confirmDuplicate, a.replaceId),
      onSuccess: (out) => {
        if (out.saved_id !== null) done();
      },
    }),
    remove: useMutation({ mutationFn: api.activityDelete, onSuccess: done }),
    revert: useMutation({ mutationFn: api.activityRevert, onSuccess: done }),
  };
}

/** Edits to market data change every value the app shows. */
export function useMarketMutations(kind: SeriesKindName, key: string) {
  const qc = useQueryClient();
  const done = () => valuesChanged(qc);
  return {
    set: useMutation({
      mutationFn: (a: { period: string; value: string; currency: string | null }) => api.marketSetValue(kind, key, a.period, a.value, a.currency),
      onSuccess: done,
    }),
    ignore: useMutation({ mutationFn: (period: string) => api.marketIgnore(kind, key, period), onSuccess: done }),
    revert: useMutation({ mutationFn: (period: string) => api.marketRevert(kind, key, period), onSuccess: done }),
    exportCsv: useMutation({ mutationFn: (dir: string) => api.marketExportCsv(kind, key, dir) }),
  };
}

export const useResetPreview = (scope: ResetScopeName) =>
  useQuery({ queryKey: ["reset-preview", scope], queryFn: () => api.resetPreview(scope) });

/** A reset changes everything: refetch every query. */
export function useResetData() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (a: { scope: ResetScopeName; confirm: string }) => api.resetData(a.scope, a.confirm),
    onSuccess: () => void qc.invalidateQueries(),
  });
}

export const useAccountsList = () => useQuery({ queryKey: keys.accountsList, queryFn: api.accountsList });
export const useAccountDetail = (id: number) => useQuery({ queryKey: [...keys.accountDetail, id], queryFn: () => api.accountDetail(id) });
export const useAccountChart = (id: number, period: ChartPeriod, currency: string) =>
  useQuery({ queryKey: [...keys.accountChart, id, period, currency], queryFn: () => api.accountChart(id, period, currency), placeholderData: (prev) => prev });

/** Account changes touch every screen that groups by account. */
export function useAccountMutations() {
  const qc = useQueryClient();
  const done = () => {
    dataChanged(qc);
    valuesChanged(qc);
  };
  return {
    rename: useMutation({ mutationFn: (a: { id: number; name: string }) => api.accountRename(a.id, a.name), onSuccess: done }),
    addIdentifier: useMutation({ mutationFn: (a: { id: number; identifier: string }) => api.accountAddIdentifier(a.id, a.identifier), onSuccess: done }),
    moveIdentifier: useMutation({ mutationFn: (a: { identifier: string; to: number }) => api.accountMoveIdentifier(a.identifier, a.to), onSuccess: done }),
    remove: useMutation({ mutationFn: api.accountDelete, onSuccess: done }),
    merge: useMutation({ mutationFn: (a: { from: number; into: number }) => api.accountMerge(a.from, a.into), onSuccess: done }),
    undoMerge: useMutation({ mutationFn: api.accountUndoMerge, onSuccess: done }),
    setTreatment: useMutation({
      mutationFn: (a: { id: number; treatment: TaxTreatment }) => api.setTaxTreatment(a.id, a.treatment),
      onSuccess: done,
    }),
    secondCurrency: useMutation({ mutationFn: api.setSecondCurrency, onSuccess: done }),
    priceSource: useMutation({ mutationFn: (a: { id: number; provider: string | null }) => api.accountSetPriceSource(a.id, a.provider), onSuccess: done }),
  };
}

/** What a merge of `from` would move; fetched once a target is chosen. */
export const useMergePreview = (from: number, enabled: boolean) =>
  useQuery({ queryKey: [...keys.accountDetail, from, "merge-preview"], queryFn: () => api.accountMergePreview(from), enabled });
