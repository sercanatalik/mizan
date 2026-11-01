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
  cmd_overview: { accounts: [], attention: [] },
  cmd_holdings: { accounts: [] },
  cmd_contributions: {
    periods: ["ytd", "last12_months", "since_start"].map((period) => ({ period, to: "2026-11-01", accounts: [], total: [] })),
  },
  cmd_import_current: null,
  cmd_import_history: [],
  cmd_accounts: [],
};
