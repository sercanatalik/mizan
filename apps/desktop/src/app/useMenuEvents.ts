import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef } from "react";
import { useNavigate } from "react-router";
import { useRefreshPrices } from "../bridge/queries";
import { usePalette } from "./palette";
import { sectionPath } from "./sections";
import { useView } from "./view";
import type { AddActivityRequest } from "./ViewBar";

export const MENU_IMPORT = "menu:import";
export const MENU_NAVIGATE = "menu:navigate";
/** Window event that toggles the Holdings inspector. */
export const TOGGLE_INSPECTOR = "mizan:toggle-inspector";

/** Location state asking Exports to start a database backup. */
export interface BackupRequest {
  backup: number;
}

/** Location state asking Imports to confirm undoing the most recent import. */
export interface UndoLastRequest {
  undoLast: number;
}

/** Location state that asks the Imports screen to open its file picker. */
export interface PickRequest {
  pick: number;
}

/** Location state that asks the Imports screen to open the column mapper for a file. */
export interface MapRequest {
  map: { path: string; fileName: string; fileKind: string; at: number };
}

/** Forward native menu clicks to the router and the app's actions. */
export function useMenuEvents() {
  const navigate = useNavigate();
  const palette = usePalette();
  const { view, setView } = useView();
  const refresh = useRefreshPrices();
  // Handlers read the latest values without re-subscribing on every change.
  const latest = useRef({ view, setView, refresh, palette });
  useEffect(() => {
    latest.current = { view, setView, refresh, palette };
  });
  useEffect(() => {
    let stopped = false;
    const unlisten: (() => void)[] = [];
    const add = (p: Promise<() => void>) =>
      p.then((u) => (stopped ? u() : unlisten.push(u))).catch(() => {
        // Not running inside Tauri (e.g. a plain browser): no native menu.
      });
    add(
      listen<string>(MENU_NAVIGATE, (e) => {
        const path = sectionPath(e.payload);
        if (path) navigate(path);
      }),
    );
    add(listen(MENU_IMPORT, () => navigate("/imports", { state: { pick: Date.now() } satisfies PickRequest })));
    add(listen("menu:find", () => latest.current.palette.show()));
    add(
      listen("menu:add-activity", () =>
        navigate("/activity", { state: { add: { accountId: latest.current.view.account_id, at: Date.now() } } satisfies AddActivityRequest }),
      ),
    );
    add(listen("menu:exports", () => navigate("/exports")));
    add(listen("menu:backup", () => navigate("/exports", { state: { backup: Date.now() } satisfies BackupRequest })));
    add(listen("menu:toggle-real", () => latest.current.setView({ real: !latest.current.view.real })));
    add(listen("menu:inspector", () => window.dispatchEvent(new Event(TOGGLE_INSPECTOR))));
    add(listen("menu:refresh", () => latest.current.refresh.mutate()));
    add(listen("menu:undo-import", () => navigate("/imports", { state: { undoLast: Date.now() } satisfies UndoLastRequest })));
    return () => {
      stopped = true;
      unlisten.forEach((u) => u());
    };
  }, [navigate]);
}
