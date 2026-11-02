import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../bridge/api";
import type { Period } from "../bridge/generated/Period";
import type { ViewState } from "../bridge/generated/ViewState";
import { keys, useAccounts } from "../bridge/queries";

export const DEFAULT_VIEW: ViewState = { account_id: null, period: "ytd", real: false };

export const PERIODS: { value: Period; label: string; long: string }[] = [
  { value: "one_month", label: "1M", long: "in the last month" },
  { value: "three_months", label: "3M", long: "in the last three months" },
  { value: "ytd", label: "YTD", long: "this year" },
  { value: "last12_months", label: "1Y", long: "in the last year" },
  { value: "since_start", label: "Since start", long: "since the first activity" },
];

/**
 * The view bar's shared choices (scope, period, Nominal | Real), kept in the
 * database. Every screen reading them shares one cached copy, so a change on
 * one screen is seen on the next.
 */
export function useView() {
  const qc = useQueryClient();
  const q = useQuery({ queryKey: keys.view, queryFn: api.viewGet, staleTime: Infinity });
  const view = q.data ?? DEFAULT_VIEW;
  const save = useMutation({
    mutationFn: api.viewSet,
    onMutate: (v) => qc.setQueryData(keys.view, v),
    onSuccess: (v) => qc.setQueryData(keys.view, v),
    onError: () => void qc.invalidateQueries({ queryKey: keys.view }),
  });
  const setView = (patch: Partial<ViewState>) => save.mutate({ ...view, ...patch });
  return { view, setView, ready: q.isSuccess };
}

/** "All accounts", or the scoped account's name. */
export function useScopeName(): string {
  const { view } = useView();
  const accounts = useAccounts().data;
  if (view.account_id === null) return "All accounts";
  return accounts?.find((a) => a.id === view.account_id)?.name ?? "One account";
}

/**
 * First day of `period` ending on `today` (ISO dates), as the core computes it:
 * one, three or twelve months back plus a day, or 1 January; null since start.
 */
export function periodStart(period: Period, today: string): string | null {
  const [y, m, d] = today.split("-").map(Number) as [number, number, number];
  const back = (months: number) => {
    // Clamp to the month's last day, as chrono does, then add a day.
    const target = new Date(Date.UTC(y, m - 1 - months, 1));
    const last = new Date(Date.UTC(target.getUTCFullYear(), target.getUTCMonth() + 1, 0)).getUTCDate();
    target.setUTCDate(Math.min(d, last) + 1);
    return target.toISOString().slice(0, 10);
  };
  switch (period) {
    case "one_month":
      return back(1);
    case "three_months":
      return back(3);
    case "ytd":
      return `${y}-01-01`;
    case "last12_months":
      return back(12);
    case "since_start":
      return null;
  }
}
