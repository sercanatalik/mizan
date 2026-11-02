import { useEffect, useRef, useState } from "react";
import { useNavigate } from "react-router";
import { useAccounts, useSettings, useSettingsMutations } from "../bridge/queries";
import { Segmented } from "../ui";
import type { PickRequest } from "./useMenuEvents";
import { PERIODS, useView } from "./view";
import s from "./layout.module.css";

/** Which view controls a screen uses; a control a screen ignores is not shown. */
export interface ViewControls {
  scope?: boolean;
  period?: boolean;
  currency?: boolean;
  real?: boolean;
}

/** Location state asking Activity to open its form. */
export interface AddActivityRequest {
  add: { accountId: number | null; at: number; kind?: "position_in"; assetId?: number | null };
}

/** Location state asking Assets to open one asset's price controls. */
export interface EditAssetRequest {
  editAsset: number;
}

/** Location state asking Assets to open the manual price form. */
export interface ManualPriceRequest {
  manualPrice: number;
}

const MODES = [
  { value: "nominal" as const, label: "Nominal" },
  { value: "real" as const, label: "Real" },
];

export function ViewBar({ controls }: { controls: ViewControls }) {
  const { view, setView } = useView();
  const accounts = useAccounts().data ?? [];
  const settings = useSettings().data;
  const { setBaseCurrency } = useSettingsMutations();
  return (
    <div role="toolbar" aria-label="View" className={s.viewBar}>
      {controls.scope && (
        <label className={s.viewField}>
          <span className={s.viewLabel}>Scope</span>
          <select
            className={s.viewSelect}
            value={view.account_id ?? ""}
            onChange={(e) => setView({ account_id: e.target.value === "" ? null : Number(e.target.value) })}
          >
            <option value="">All accounts</option>
            {accounts.map((a) => (
              <option key={a.id} value={a.id}>
                {a.name}
              </option>
            ))}
          </select>
        </label>
      )}
      {controls.period && <Segmented label="Period" value={view.period} onChange={(period) => setView({ period })} options={PERIODS} />}
      {controls.currency && settings && (
        <label className={s.viewField}>
          <span className={s.viewLabel}>Currency</span>
          <select className={s.viewSelect} value={settings.base_currency} onChange={(e) => setBaseCurrency.mutate(e.target.value)} disabled={setBaseCurrency.isPending}>
            {settings.currencies.map((c) => (
              <option key={c} value={c}>
                {c}
              </option>
            ))}
          </select>
        </label>
      )}
      {controls.real && <Segmented label="Returns" value={view.real ? "real" : "nominal"} onChange={(m) => setView({ real: m === "real" })} options={MODES} />}
      <span className={s.viewSpacer} />
      <AddMenu accountId={view.account_id} />
    </div>
  );
}

/** Import file…, Add activity and Add manual price. */
export function AddMenu({ accountId }: { accountId: number | null }) {
  const [open, setOpen] = useState(false);
  const navigate = useNavigate();
  const box = useRef<HTMLDivElement>(null);
  const first = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (!open) return;
    first.current?.focus();
    const away = (e: MouseEvent) => {
      if (box.current && !box.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", away);
    return () => document.removeEventListener("mousedown", away);
  }, [open]);
  const importFile = () => {
    setOpen(false);
    navigate("/imports", { state: { pick: Date.now() } satisfies PickRequest });
  };
  const addActivity = () => {
    setOpen(false);
    navigate("/activity", { state: { add: { accountId, at: Date.now() } } satisfies AddActivityRequest });
  };
  const addPrice = () => {
    setOpen(false);
    navigate("/assets", { state: { manualPrice: Date.now() } satisfies ManualPriceRequest });
  };
  return (
    <div
      ref={box}
      className={s.addMenu}
      onKeyDown={(e) => {
        if (e.key === "Escape") setOpen(false);
      }}
    >
      <button type="button" className={s.addButton} aria-haspopup="menu" aria-expanded={open} onClick={() => setOpen((o) => !o)}>
        <svg aria-hidden="true" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round">
          <path d="M12 5v14M5 12h14" />
        </svg>
        Add
      </button>
      {open && (
        <div role="menu" aria-label="Add" className={s.menu}>
          <button
            ref={first}
            type="button"
            role="menuitem"
            className={s.menuItem}
            onClick={importFile}
          >
            Import file…
          </button>
          <button
            type="button"
            role="menuitem"
            className={s.menuItem}
            onClick={addActivity}
          >
            Add activity
          </button>
          <button
            type="button"
            role="menuitem"
            className={s.menuItem}
            onClick={addPrice}
          >
            Add manual price
          </button>
        </div>
      )}
    </div>
  );
}
