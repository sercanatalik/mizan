// Typed wrappers over the Tauri commands. Types come from the Rust side (./generated).
import { invoke } from "@tauri-apps/api/core";
import type { AccountView } from "./generated/AccountView";
import type { BatchView } from "./generated/BatchView";
import type { CommitView } from "./generated/CommitView";
import type { Contributions } from "./generated/Contributions";
import type { Decision } from "./generated/Decision";
import type { Holdings } from "./generated/Holdings";
import type { ImportView } from "./generated/ImportView";
import type { Overview } from "./generated/Overview";

/** Errors from the Rust side arrive as `{ kind, message }`. */
export interface AppError {
  kind: "io" | "batch" | "storage" | "commit" | "invalid";
  message: string;
}

export function errorMessage(e: unknown): string {
  if (e && typeof e === "object" && "message" in e && typeof e.message === "string") return e.message;
  return String(e);
}

export const api = {
  importOpen: (path: string) => invoke<ImportView>("cmd_import_open", { path }),
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
};
