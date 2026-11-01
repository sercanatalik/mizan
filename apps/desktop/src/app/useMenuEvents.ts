import { listen } from "@tauri-apps/api/event";
import { useEffect } from "react";
import { useNavigate } from "react-router";
import { sectionPath } from "./sections";

export const MENU_IMPORT = "menu:import";
export const MENU_NAVIGATE = "menu:navigate";

/** Location state that asks the Imports screen to open its file picker. */
export interface PickRequest {
  pick: number;
}

/** Forward native menu clicks to the router. */
export function useMenuEvents() {
  const navigate = useNavigate();
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
    return () => {
      stopped = true;
      unlisten.forEach((u) => u());
    };
  }, [navigate]);
}
