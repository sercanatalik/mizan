import { useQueryClient } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import { useEffect } from "react";
import { keys, valuesChanged } from "../bridge/queries";

export const REFRESH_STARTED = "market:refresh-started";
export const REFRESH_FINISHED = "market:refresh-finished";

/** Track background price refreshes started by the app (for example on launch). */
export function useMarketEvents() {
  const qc = useQueryClient();
  useEffect(() => {
    let stopped = false;
    const unlisten: (() => void)[] = [];
    const add = (p: Promise<() => void>) =>
      p.then((u) => (stopped ? u() : unlisten.push(u))).catch(() => {
        // Not running inside Tauri.
      });
    add(listen(REFRESH_STARTED, () => qc.setQueryData(keys.refreshing, true)));
    add(
      listen(REFRESH_FINISHED, () => {
        qc.setQueryData(keys.refreshing, false);
        valuesChanged(qc);
      }),
    );
    return () => {
      stopped = true;
      unlisten.forEach((u) => u());
    };
  }, [qc]);
}
