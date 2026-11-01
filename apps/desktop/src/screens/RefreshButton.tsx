import { useQuery } from "@tanstack/react-query";
import { errorMessage } from "../bridge/api";
import { keys, useRefreshPrices } from "../bridge/queries";
import { Button } from "../ui";

/** "Refresh prices" action; also reflects background refreshes started by the app. */
export function RefreshButton() {
  const refresh = useRefreshPrices();
  const background = useQuery({ queryKey: keys.refreshing, queryFn: () => false, staleTime: Infinity }).data ?? false;
  const busy = refresh.isPending || background;
  return (
    <span style={{ display: "inline-flex", alignItems: "center", gap: 8 }}>
      {refresh.error && (
        <span role="alert" style={{ fontSize: 12, color: "var(--warn-strong)" }}>
          {errorMessage(refresh.error)}
        </span>
      )}
      <Button onClick={() => refresh.mutate()} disabled={busy} aria-busy={busy}>
        {busy ? "Refreshing prices…" : "Refresh prices"}
      </Button>
    </span>
  );
}
