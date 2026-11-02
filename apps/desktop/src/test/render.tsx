import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render } from "@testing-library/react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { routes } from "../app/routes";

/** Render the whole app at `path` with a fresh query cache. */
export function renderApp(path = "/") {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const router = createMemoryRouter(routes, { initialEntries: [path] });
  const utils = render(
    <QueryClientProvider client={qc}>
      <RouterProvider router={router} />
    </QueryClientProvider>,
  );
  return { ...utils, router, qc };
}

type Responses = Record<string, unknown>;

/** An `invoke` mock answering each command from `responses` (functions get the args). */
export function invokeFrom(responses: Responses) {
  return async (cmd: string, args?: unknown) => {
    if (!(cmd in responses)) throw new Error(`unexpected command ${cmd}`);
    const r = responses[cmd];
    return typeof r === "function" ? (r as (a: unknown) => unknown)(args) : r;
  };
}

export const EMPTY = {
  cmd_pending_decisions: 0,
  cmd_overview: { base_currency: "TRY", accounts: [], attention: [], total: { currency: "TRY", amount: "0.00", excluded_no_price: 0, excluded_no_rate: 0, oldest_price: null }, has_prices: false },
  cmd_holdings: { base_currency: "TRY", accounts: [], total: { currency: "TRY", amount: "0.00", excluded_no_price: 0, excluded_no_rate: 0, oldest_price: null }, has_prices: false },
  cmd_contributions: {
    periods: ["one_month", "three_months", "ytd", "last12_months", "since_start"].map((period) => ({ period, to: "2026-11-01", accounts: [], total: [] })),
  },
  cmd_import_current: null,
  cmd_import_history: [],
  cmd_accounts: [],
  cmd_assets: [],
  cmd_value_history: { base_currency: "TRY", points: [], valued_days: 0 },
  cmd_backfill_status: {
    cache_path: "/Users/a/.mizan/market-history.sqlite",
    sources: [
      { id: "tefas", name: "TEFAS", state: "idle", done: 0, planned: 0, unit: "months", error: null, note: null },
      { id: "tcmb", name: "TCMB", state: "idle", done: 0, planned: 0, unit: "days", error: null, note: null },
      { id: "yahoo", name: "Yahoo Finance", state: "idle", done: 0, planned: 0, unit: "years", error: null, note: null },
      { id: "twelvedata", name: "Twelve Data", state: "idle", done: 0, planned: 0, unit: "years", error: null, note: null },
    ],
  },
  cmd_settings: {
    base_currency: "TRY",
    currencies: ["TRY", "USD", "EUR"],
    refresh_on_start: true,
    last_refresh_at: null,
    providers: [
      { id: "tefas", name: "TEFAS", supplies: "Turkish investment and pension fund prices (TRY)", enabled: true, last_success: null, needs_api_key: false, api_key_hint: null },
      { id: "tcmb", name: "TCMB", supplies: "Exchange rates from the Turkish central bank", enabled: true, last_success: null, needs_api_key: false, api_key_hint: null },
      {
        id: "yahoo",
        name: "Yahoo Finance",
        supplies: "US and European stock and ETF prices; unofficial source that may change without notice",
        enabled: true,
        last_success: null,
        needs_api_key: false,
        api_key_hint: null,
      },
      {
        id: "twelvedata",
        name: "Twelve Data",
        supplies: "Stock and ETF prices with your free API key; the free plan covers US listings",
        enabled: true,
        last_success: null,
        needs_api_key: true,
        api_key_hint: null,
      },
    ],
    last_report: null,
    refreshing: false,
  },
};
