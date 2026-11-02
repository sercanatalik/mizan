// Typed wrappers over the Tauri commands. Types come from the Rust side (./generated).
import { invoke } from "@tauri-apps/api/core";
import type { AccountView } from "./generated/AccountView";
import type { AssetView } from "./generated/AssetView";
import type { BackfillProgress } from "./generated/BackfillProgress";
import type { ViewState } from "./generated/ViewState";
import type { InboxView } from "./generated/InboxView";
import type { AdaptersList } from "./generated/AdaptersList";
import type { IncomeView } from "./generated/IncomeView";
import type { BatchView } from "./generated/BatchView";
import type { CommitView } from "./generated/CommitView";
import type { Contributions } from "./generated/Contributions";
import type { Decision } from "./generated/Decision";
import type { Holdings } from "./generated/Holdings";
import type { ImportView } from "./generated/ImportView";
import type { MapperInspect } from "./generated/MapperInspect";
import type { MapperOptions } from "./generated/MapperOptions";
import type { MapperPreview } from "./generated/MapperPreview";
import type { OpenResult } from "./generated/OpenResult";
import type { SavedMapping } from "./generated/SavedMapping";
import type { Overview } from "./generated/Overview";
import type { PerformanceView } from "./generated/PerformanceView";
import type { Period } from "./generated/Period";
import type { ValueHistory } from "./generated/ValueHistory";
import type { RefreshReport } from "./generated/RefreshReport";
import type { SettingsView } from "./generated/SettingsView";
import type { Country } from "./generated/Country";
import type { TaxExport } from "./generated/TaxExport";
import type { TaxReport } from "./generated/TaxReport";
import type { TaxYears } from "./generated/TaxYears";
import type { ActivityDetail } from "./generated/ActivityDetail";
import type { ActivityForm } from "./generated/ActivityForm";
import type { ActivityOptions } from "./generated/ActivityOptions";
import type { ActivityPage } from "./generated/ActivityPage";
import type { ActivityQuery } from "./generated/ActivityQuery";
import type { FormCheck } from "./generated/FormCheck";
import type { SaveOutcome } from "./generated/SaveOutcome";
import type { SeriesDetail } from "./generated/SeriesDetail";
import type { ResetOutcome } from "./generated/ResetOutcome";
import type { AccountDetail } from "./generated/AccountDetail";
import type { AccountsList } from "./generated/AccountsList";
import type { ChartPeriod } from "./generated/ChartPeriod";
import type { CurrentUnitsChart } from "./generated/CurrentUnitsChart";
import type { MergePreview } from "./generated/MergePreview";
import type { PriceSourceOutcome } from "./generated/PriceSourceOutcome";
import type { ResetPreview } from "./generated/ResetPreview";

export type ResetScopeName = "portfolio" | "everything";
import type { SeriesFilter } from "./generated/SeriesFilter";
import type { SeriesPage } from "./generated/SeriesPage";

export type SeriesKindName = "price" | "rate" | "index";

export type TaxTreatment = "taxable" | "isa" | "pension";

export type PriceSourceKind = "tefas" | "yahoo" | "twelvedata" | "manual" | "none";

/** Errors from the Rust side arrive as `{ kind, message }`. */
export interface AppError {
  kind: "io" | "batch" | "storage" | "commit" | "invalid" | "adapter";
  message: string;
}

export function errorMessage(e: unknown): string {
  if (e && typeof e === "object" && "message" in e && typeof e.message === "string") return e.message;
  return String(e);
}

export const api = {
  importOpen: (path: string) => invoke<OpenResult>("cmd_import_open", { path }),
  importChooseAdapter: (id: string) => invoke<ImportView>("cmd_import_choose_adapter", { id }),
  importCurrent: () => invoke<ImportView | null>("cmd_import_current"),
  importDecide: (index: number, decision: Decision | null) =>
    invoke<ImportView>("cmd_import_decide", { index, decision }),
  importCreateAccount: (hint: string) => invoke<ImportView>("cmd_import_create_account", { hint }),
  importMapAccount: (hint: string, accountId: number) =>
    invoke<ImportView>("cmd_import_map_account", { hint, account_id: accountId }),
  importCreateAsset: (key: string) => invoke<ImportView>("cmd_import_create_asset", { key }),
  importCommit: () => invoke<CommitView>("cmd_import_commit"),
  importClose: () => invoke<void>("cmd_import_close"),
  importHistory: () => invoke<BatchView[]>("cmd_import_history"),
  importUndo: (batchId: number) => invoke<void>("cmd_import_undo", { batch_id: batchId }),
  pendingDecisions: () => invoke<number>("cmd_pending_decisions"),
  viewGet: () => invoke<ViewState>("cmd_view_get"),
  income: (period: Period, accountId: number | null) => invoke<IncomeView>("cmd_income", { period, account_id: accountId }),
  /** Write the payments behind Income as CSV into `dir`; returns the file path. */
  exportIncomeCsv: (period: Period, accountId: number | null, dir: string) =>
    invoke<string>("cmd_export_income_csv", { period, account_id: accountId, dir }),
  /** Copy the database into `dir`; returns the backup's path. */
  backupDatabase: (dir: string) => invoke<string>("cmd_backup_database", { dir }),
  exportActivitiesCsv: (dir: string) => invoke<string>("cmd_export_activities_csv", { dir }),
  exportHoldingsCsv: (accountId: number | null, dir: string) => invoke<string>("cmd_export_holdings_csv", { account_id: accountId, dir }),
  adaptersList: () => invoke<AdaptersList>("cmd_adapters_list"),
  mappingRename: (id: string, name: string) => invoke<AdaptersList>("cmd_mapping_rename", { id, name }),
  mappingDelete: (id: string) => invoke<AdaptersList>("cmd_mapping_delete", { id }),
  /** Write an adapter's manifest (saved or bundled) to `dest`. */
  adapterExport: (id: string, dest: string) => invoke<void>("cmd_adapter_export", { id, dest }),
  inbox: () => invoke<InboxView>("cmd_inbox"),
  inboxCount: () => invoke<number>("cmd_inbox_count"),
  /** Hide an item for 1 or 7 days, or until its figures change. */
  inboxSnooze: (key: string, days: 1 | 7) => invoke<InboxView>("cmd_inbox_snooze", { key, days }),
  inboxUnsnooze: (key: string) => invoke<void>("cmd_inbox_unsnooze", { key }),
  viewSet: (view: ViewState) => invoke<ViewState>("cmd_view_set", { view }),
  holdings: (accountId: number | null) => invoke<Holdings>("cmd_holdings", { account_id: accountId }),
  overview: (accountId: number | null) => invoke<Overview>("cmd_overview", { account_id: accountId }),
  contributions: (accountId: number | null) => invoke<Contributions>("cmd_contributions", { account_id: accountId }),
  accounts: () => invoke<AccountView[]>("cmd_accounts"),
  // Returns
  performance: (period: Period, accountId: number | null) =>
    invoke<PerformanceView>("cmd_performance", { period, account_id: accountId }),
  valueHistory: (accountId: number | null) => invoke<ValueHistory>("cmd_value_history", { account_id: accountId }),
  setBenchmark: (assetId: number | null) => invoke<void>("cmd_set_benchmark", { asset_id: assetId }),
  // Activity
  activities: (query: ActivityQuery, offset: number) => invoke<ActivityPage>("cmd_activities", { query, offset }),
  activityDetail: (id: number) => invoke<ActivityDetail>("cmd_activity_detail", { id }),
  activityOptions: () => invoke<ActivityOptions>("cmd_activity_options"),
  activityCheck: (form: ActivityForm, replaceId: number | null) => invoke<FormCheck>("cmd_activity_check", { form, replace_id: replaceId }),
  activitySave: (form: ActivityForm, confirmDuplicate: boolean, replaceId: number | null) =>
    invoke<SaveOutcome>("cmd_activity_save", { form, confirm_duplicate: confirmDuplicate, replace_id: replaceId }),
  activityDelete: (id: number) => invoke<void>("cmd_activity_delete", { id }),
  /** Restore an amended imported activity's imported values. */
  activityRevert: (id: number) => invoke<void>("cmd_activity_revert", { id }),
  // Market data: the shared cache is only read; edits stay in this database.
  marketSeries: (kind: SeriesKindName, filter: SeriesFilter) => invoke<SeriesPage>("cmd_market_series", { kind, filter }),
  marketSeriesDetail: (kind: SeriesKindName, key: string, offset: number) =>
    invoke<SeriesDetail>("cmd_market_series_detail", { kind, key, offset }),
  marketSetValue: (kind: SeriesKindName, key: string, period: string, value: string, currency: string | null) =>
    invoke<void>("cmd_market_set_value", { kind, key, period, value, currency }),
  marketIgnore: (kind: SeriesKindName, key: string, period: string) => invoke<void>("cmd_market_ignore", { kind, key, period }),
  marketRevert: (kind: SeriesKindName, key: string, period: string) => invoke<void>("cmd_market_revert", { kind, key, period }),
  /** Write the series as CSV into `dir`; returns the file path. */
  marketExportCsv: (kind: SeriesKindName, key: string, dir: string) => invoke<string>("cmd_market_export_csv", { kind, key, dir }),
  // Accounts
  accountsList: () => invoke<AccountsList>("cmd_accounts_list"),
  accountDetail: (id: number) => invoke<AccountDetail>("cmd_account_detail", { id }),
  accountChart: (id: number, period: ChartPeriod, currency: string) => invoke<CurrentUnitsChart>("cmd_account_chart", { id, period, currency }),
  accountRename: (id: number, name: string) => invoke<void>("cmd_account_rename", { id, name }),
  accountAddIdentifier: (id: number, identifier: string) => invoke<void>("cmd_account_add_identifier", { id, identifier }),
  accountMoveIdentifier: (identifier: string, to: number) => invoke<void>("cmd_account_move_identifier", { identifier, to }),
  accountDelete: (id: number) => invoke<void>("cmd_account_delete", { id }),
  accountMergePreview: (from: number) => invoke<MergePreview>("cmd_account_merge_preview", { from }),
  accountMerge: (from: number, into: number) => invoke<number>("cmd_account_merge", { from, into }),
  /** Returns identifiers that had moved elsewhere and stayed there. */
  accountUndoMerge: (mergeId: number) => invoke<string[]>("cmd_account_undo_merge", { merge_id: mergeId }),
  /** `provider` is `tefas`, `yahoo`, `twelvedata`, or null for per asset. */
  accountSetPriceSource: (id: number, provider: string | null) => invoke<PriceSourceOutcome>("cmd_account_set_price_source", { id, provider }),
  setSecondCurrency: (currency: string) => invoke<void>("cmd_set_second_currency", { currency }),
  // Reset: a backup is written first; `confirm` must be "RESET".
  resetPreview: (scope: ResetScopeName) => invoke<ResetPreview>("cmd_reset_preview", { scope }),
  resetData: (scope: ResetScopeName, confirm: string) => invoke<ResetOutcome>("cmd_reset_data", { scope, confirm }),
  // Tax reports: computed from stored data only.
  taxYears: (country: Country) => invoke<TaxYears>("cmd_tax_years", { country }),
  taxReport: (country: Country, year: string) => invoke<TaxReport>("cmd_tax_report", { country, year }),
  setTaxTreatment: (accountId: number, treatment: TaxTreatment) =>
    invoke<void>("cmd_set_tax_treatment", { account_id: accountId, treatment }),
  /** Write the gains and income CSV files into `dir`. */
  exportTaxCsv: (country: Country, year: string, dir: string) => invoke<TaxExport>("cmd_export_tax_csv", { country, year, dir }),
  // Column mapper. `mapping` is the mapping file's TOML document as JSON.
  mapperInspect: (path: string, options: MapperOptions | null) => invoke<MapperInspect>("cmd_mapper_inspect", { path, options }),
  mapperPreview: (path: string, mapping: object) => invoke<MapperPreview>("cmd_mapper_preview", { path, mapping }),
  mapperSave: (mapping: object) => invoke<SavedMapping>("cmd_mapper_save", { mapping }),
  mapperExport: (id: string, dest: string) => invoke<void>("cmd_mapper_export", { id, dest }),
  // Market data
  refreshPrices: () => invoke<RefreshReport>("cmd_refresh_prices"),
  backfillStatus: () => invoke<BackfillProgress>("cmd_backfill_status"),
  settings: () => invoke<SettingsView>("cmd_settings"),
  setProviderEnabled: (id: string, enabled: boolean) => invoke<void>("cmd_set_provider_enabled", { id, enabled }),
  /** Save a provider's API key, or clear it with null. The key is never returned. */
  setApiKey: (id: string, key: string | null) => invoke<void>("cmd_set_api_key", { id, key }),
  setBaseCurrency: (currency: string) => invoke<void>("cmd_set_base_currency", { currency }),
  setRefreshOnStart: (enabled: boolean) => invoke<void>("cmd_set_refresh_on_start", { enabled }),
  assets: () => invoke<AssetView[]>("cmd_assets"),
  /** `symbol`: TEFAS fund code or provider symbol; null for manual and none. */
  setPriceSource: (assetId: number, kind: PriceSourceKind, symbol: string | null) =>
    invoke<void>("cmd_set_price_source", { asset_id: assetId, kind, symbol }),
  acceptSuggestion: (assetId: number) => invoke<void>("cmd_accept_suggestion", { asset_id: assetId }),
  /** Mark an asset as a benchmark (history downloaded in full, offered on Performance). */
  setBenchmarkCandidate: (assetId: number, on: boolean) => invoke<void>("cmd_set_benchmark_candidate", { asset_id: assetId, on }),
  /** Add an instrument to compare against, held or not; returns its asset id. */
  addBenchmarkInstrument: (kind: "tefas" | "yahoo" | "twelvedata", symbol: string, name: string | null) =>
    invoke<number>("cmd_add_benchmark_instrument", { kind, symbol, name }),
  addManualPrice: (assetId: number, date: string, value: string, currency: string) =>
    invoke<void>("cmd_add_manual_price", { asset_id: assetId, date, value, currency }),
  deleteManualPrice: (assetId: number, date: string) => invoke<void>("cmd_delete_manual_price", { asset_id: assetId, date }),
  mergeAssets: (from: number, into: number) => invoke<number>("cmd_merge_assets", { from, into }),
  undoMerge: (mergeId: number) => invoke<void>("cmd_undo_merge", { merge_id: mergeId }),
};
