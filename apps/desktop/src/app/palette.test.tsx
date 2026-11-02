import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { EMPTY, invokeFrom, renderApp } from "../test/render";
import { fold, matchGroups } from "./palette";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
const mockInvoke = vi.mocked(invoke);
vi.mocked(listen);

beforeEach(() => {
  mockInvoke.mockReset();
  mockInvoke.mockImplementation(invokeFrom(EMPTY) as typeof invoke);
});

test("the sidebar search button opens the palette", async () => {
  renderApp();
  const nav = screen.getByRole("navigation", { name: "Main" });
  await userEvent.click(within(nav).getByRole("button", { name: /Search or run/ }));
  const dialog = screen.getByRole("dialog", { name: "Search or run a command" });
  expect(within(dialog).getByRole("combobox", { name: "Search" })).toHaveFocus();
  expect(within(dialog).getByRole("group", { name: "Go to" })).toBeInTheDocument();
});

test("⌘K opens, Escape closes and returns focus", async () => {
  renderApp();
  const button = within(screen.getByRole("navigation", { name: "Main" })).getByRole("button", { name: /Search or run/ });
  button.focus();
  await userEvent.keyboard("{Meta>}k{/Meta}");
  expect(screen.getByRole("dialog")).toBeInTheDocument();
  await userEvent.keyboard("{Escape}");
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(button).toHaveFocus();
});

test("typing and Enter opens a screen", async () => {
  const { router } = renderApp();
  await userEvent.keyboard("{Control>}k{/Control}");
  await userEvent.keyboard("inb{Enter}");
  await waitFor(() => expect(router.state.location.pathname).toBe("/inbox"));
  expect(screen.queryByRole("dialog")).toBeNull();
});

test("no match says so", async () => {
  renderApp();
  await userEvent.keyboard("{Meta>}k{/Meta}");
  await userEvent.keyboard("zzzz");
  expect(screen.getByText(/No match/)).toBeInTheDocument();
});

test("folding treats Turkish letters as plain Latin", () => {
  expect(fold("İş Portföy BIST Teknoloji")).toBe("is portfoy bist teknoloji");
  expect(fold("Işık Çağ")).toBe("isik cag");
});

test("matching ranks label prefixes first and hides empty groups", () => {
  const run = () => {};
  const groups = [
    { name: "A", items: [{ id: "1", label: "Market data", run }, { id: "2", label: "Data reset", hint: "settings", run }] },
    { name: "B", items: [{ id: "3", label: "Holdings", run }] },
  ];
  const r = matchGroups(groups, "data");
  expect(r.map((g) => g.name)).toEqual(["A"]);
  expect(r[0]!.items.map((i) => i.label)).toEqual(["Data reset", "Market data"]);
});

const asset = (asset_id: number, label: string, name: string | null, instrument_key: string, merged = false) => ({
  asset_id,
  instrument_key,
  label,
  name,
  class: "fund",
  currency: "TRY",
  source: { kind: "none", symbol: null },
  suggestion: null,
  latest_price: null,
  holders: 1,
  merged_into: merged ? { merge_id: 1, asset_id: 1, label: "X" } : null,
  manual_prices: [],
});

function serveData(history: unknown[] = []) {
  mockInvoke.mockImplementation(
    invokeFrom({
      ...EMPTY,
      cmd_accounts: [
        { id: 4, name: "IBKR", hints: ["U1234567"] },
        { id: 5, name: "Pension", hints: ["P1"] },
      ],
      cmd_assets: [asset(1, "AFT", "Ak Portföy Yeni Teknolojiler", "sym:AFT@"), asset(2, "TTE", "İş Portföy BIST Teknoloji", "sym:TTE@"), asset(3, "OLD", null, "sym:OLD@", true)],
      cmd_import_history: history,
    }) as typeof invoke,
  );
}

const groupsShown = () => within(screen.getByRole("dialog")).getAllByRole("group").map((g) => g.getAttribute("aria-label"));

test("empty query shows Actions and Go to; Undo is hidden without imports", async () => {
  serveData();
  renderApp();
  await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("cmd_import_history"));
  await userEvent.keyboard("{Meta>}k{/Meta}");
  expect(groupsShown()).toEqual(["Actions", "Go to"]);
  const actions = within(screen.getByRole("group", { name: "Actions" })).getAllByRole("option").map((o) => o.textContent);
  expect(actions.some((a) => a?.startsWith("Import file…"))).toBe(true);
  expect(actions.some((a) => a?.startsWith("Add activity"))).toBe(true);
  expect(actions.some((a) => a?.startsWith("Add manual price"))).toBe(true);
  expect(actions.some((a) => a?.startsWith("Refresh prices"))).toBe(true);
  expect(actions.some((a) => a?.startsWith("Switch to real returns"))).toBe(true);
  expect(actions.some((a) => a?.startsWith("Undo last import"))).toBe(false);
});

test("Undo last import is offered once an import exists", async () => {
  serveData([{ id: 1, committed_at: "2026-10-01T10:00:00Z", adapter: "ofx", adapter_version: "1", file_sha256: "a".repeat(64), activity_count: 3, assertion_count: 0, all_checks_passed: null }]);
  renderApp();
  await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("cmd_import_history"));
  await userEvent.keyboard("{Meta>}k{/Meta}");
  await userEvent.keyboard("undo");
  expect(within(screen.getByRole("dialog")).getByRole("option", { name: /Undo last import/ })).toBeInTheDocument();
});

test("typing a symbol finds the asset and hides groups without a match", async () => {
  serveData();
  const { router } = renderApp();
  await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("cmd_assets"));
  await userEvent.keyboard("{Meta>}k{/Meta}");
  await userEvent.keyboard("aft");
  expect(groupsShown()[0]).toBe("Assets");
  expect(groupsShown()).not.toContain("Accounts");
  expect(groupsShown()).not.toContain("Go to");
  expect(within(screen.getByRole("group", { name: "Assets" })).getByRole("option", { name: /AFT/ })).toHaveAttribute("aria-selected", "true");
  expect(within(screen.getByRole("dialog")).queryByText("OLD")).toBeNull();
  await userEvent.keyboard("{Enter}");
  await waitFor(() => expect(router.state.location.pathname).toBe("/assets"));
  expect(router.state.location.state).toEqual({ editAsset: 1 });
});

test("Turkish letters match their plain forms", async () => {
  serveData();
  renderApp();
  await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("cmd_assets"));
  await userEvent.keyboard("{Meta>}k{/Meta}");
  await userEvent.keyboard("is portfoy");
  expect(within(screen.getByRole("dialog")).getByRole("option", { name: /İş Portföy BIST Teknoloji/ })).toBeInTheDocument();
});

test("an account opens its page with Enter; identifiers match too", async () => {
  serveData();
  const { router } = renderApp();
  await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("cmd_accounts"));
  await userEvent.keyboard("{Meta>}k{/Meta}");
  await userEvent.keyboard("u1234");
  expect(within(screen.getByRole("dialog")).getByRole("option", { name: "IBKR" })).toBeInTheDocument();
  await userEvent.clear(screen.getByRole("combobox", { name: "Search" }));
  await userEvent.keyboard("ibkr{Enter}");
  await waitFor(() => expect(router.state.location.pathname).toBe("/accounts/4"));
  expect(screen.queryByRole("dialog")).toBeNull();
});

test("arrow keys move the selection", async () => {
  serveData();
  renderApp();
  await userEvent.keyboard("{Meta>}k{/Meta}");
  const options = within(screen.getByRole("dialog")).getAllByRole("option");
  expect(options[0]).toHaveAttribute("aria-selected", "true");
  await userEvent.keyboard("{ArrowDown}");
  expect(options[1]).toHaveAttribute("aria-selected", "true");
  await userEvent.keyboard("{ArrowUp}");
  expect(options[0]).toHaveAttribute("aria-selected", "true");
});
