// Typed wrappers over the Tauri commands. Types come from the Rust side (./generated).
import { invoke } from "@tauri-apps/api/core";
import type { AccountView } from "./generated/AccountView";
import type { AssetView } from "./generated/AssetView";
import type { BackfillProgress } from "./generated/BackfillProgress";
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

export type PriceSourceKind = "tefas" | "manual" | "none";

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
  holdings: () => invoke<Holdings>("cmd_holdings"),
  overview: () => invoke<Overview>("cmd_overview"),
  contributions: () => invoke<Contributions>("cmd_contributions"),
  accounts: () => invoke<AccountView[]>("cmd_accounts"),
  // Returns
  performance: (period: Period) => invoke<PerformanceView>("cmd_performance", { period }),
  valueHistory: () => invoke<ValueHistory>("cmd_value_history"),
  setBenchmark: (assetId: number | null) => invoke<void>("cmd_set_benchmark", { asset_id: assetId }),
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
  setBaseCurrency: (currency: string) => invoke<void>("cmd_set_base_currency", { currency }),
  setRefreshOnStart: (enabled: boolean) => invoke<void>("cmd_set_refresh_on_start", { enabled }),
  assets: () => invoke<AssetView[]>("cmd_assets"),
  setPriceSource: (assetId: number, kind: PriceSourceKind, fundCode: string | null) =>
    invoke<void>("cmd_set_price_source", { asset_id: assetId, kind, fund_code: fundCode }),
  acceptSuggestion: (assetId: number) => invoke<void>("cmd_accept_suggestion", { asset_id: assetId }),
  addManualPrice: (assetId: number, date: string, value: string, currency: string) =>
    invoke<void>("cmd_add_manual_price", { asset_id: assetId, date, value, currency }),
  deleteManualPrice: (assetId: number, date: string) => invoke<void>("cmd_delete_manual_price", { asset_id: assetId, date }),
  mergeAssets: (from: number, into: number) => invoke<number>("cmd_merge_assets", { from, into }),
  undoMerge: (mergeId: number) => invoke<void>("cmd_undo_merge", { merge_id: mergeId }),
};
