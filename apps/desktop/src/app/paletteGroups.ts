import { useNavigate } from "react-router";
import { useAccounts, useAssets, useImportHistory, useRefreshPrices } from "../bridge/queries";
import { useGoToGroup, type PaletteGroup } from "./palette";
import type { PickRequest, UndoLastRequest } from "./useMenuEvents";
import { useView } from "./view";
import type { AddActivityRequest, EditAssetRequest, ManualPriceRequest } from "./ViewBar";

/** Everything the command palette offers: actions, screens, accounts and assets. */
export function usePaletteGroups(): PaletteGroup[] {
  const navigate = useNavigate();
  const go = useGoToGroup();
  const { view, setView } = useView();
  const refresh = useRefreshPrices();
  const accounts = useAccounts().data ?? [];
  const assets = (useAssets().data ?? []).filter((a) => !a.merged_into);
  const history = useImportHistory().data ?? [];

  const actions: PaletteGroup = {
    name: "Actions",
    always: true,
    items: [
      { id: "act:import", label: "Import file…", shortcut: "⌘O", run: () => navigate("/imports", { state: { pick: Date.now() } satisfies PickRequest }) },
      {
        id: "act:add-activity",
        label: "Add activity",
        hint: "buy, sell, dividend, transfer…",
        shortcut: "⌘N",
        run: () => navigate("/activity", { state: { add: { accountId: view.account_id, at: Date.now() } } satisfies AddActivityRequest }),
      },
      { id: "act:manual-price", label: "Add manual price", run: () => navigate("/assets", { state: { manualPrice: Date.now() } satisfies ManualPriceRequest }) },
      { id: "act:refresh", label: "Refresh prices", shortcut: "⇧⌘R", run: () => refresh.mutate() },
      view.real
        ? { id: "act:nominal", label: "Switch to nominal returns", shortcut: "⌘R", run: () => setView({ real: false }) }
        : { id: "act:real", label: "Switch to real returns", hint: "after inflation", shortcut: "⌘R", run: () => setView({ real: true }) },
      ...(history.length > 0
        ? [{ id: "act:undo-import", label: "Undo last import", run: () => navigate("/imports", { state: { undoLast: Date.now() } satisfies UndoLastRequest }) }]
        : []),
      { id: "act:providers", label: "Price providers", hint: "Market data › Providers", run: () => navigate("/market-data?tab=providers") },
    ],
  };
  const accountGroup: PaletteGroup = {
    name: "Accounts",
    items: accounts.map((a) => ({
      id: `acct:${a.id}`,
      label: a.name,
      search: a.hints.join(" "),
      run: () => navigate(`/accounts/${a.id}`),
    })),
  };
  const assetGroup: PaletteGroup = {
    name: "Assets",
    items: assets.map((a) => ({
      id: `asset:${a.asset_id}`,
      label: a.label,
      hint: a.name && a.name !== a.label ? a.name : undefined,
      search: a.instrument_key,
      run: () => navigate("/assets", { state: { editAsset: a.asset_id } satisfies EditAssetRequest }),
    })),
  };
  return [actions, go, accountGroup, assetGroup];
}
