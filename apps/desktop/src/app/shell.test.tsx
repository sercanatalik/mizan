import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { EMPTY, invokeFrom, renderApp } from "../test/render";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
const mockInvoke = vi.mocked(invoke);
const mockListen = vi.mocked(listen);
type Handler = (e: { payload: unknown }) => void;
let handlers: Record<string, Handler>;

beforeEach(() => {
  handlers = {};
  mockInvoke.mockReset();
  mockInvoke.mockImplementation(invokeFrom(EMPTY) as typeof invoke);
  mockListen.mockReset();
  mockListen.mockImplementation((async (name: string, h: Handler) => {
    handlers[name] = h;
    return () => {};
  }) as unknown as typeof listen);
});

test("sidebar lists sections in spec order without spending", () => {
  renderApp();
  const nav = screen.getByRole("navigation", { name: "Main" });
  const labels = within(nav)
    .getAllByRole("link")
    .filter((a) => !a.getAttribute("aria-label")?.startsWith("Data status"))
    .map((a) => a.textContent)
    .filter((t) => t !== "mizan");
  expect(labels).toEqual([
    "Overview",
    "Inbox",
    "Holdings",
    "Accounts",
    "Performance",
    "Income",
    "Activity",
    "Imports",
    "Adapters",
    "Tax",
    "Exports",
    "Assets",
    "Market data",
    "Settings",
  ]);
  expect(within(nav).queryByText(/spending|budget/i)).toBeNull();
  for (const g of ["Portfolio", "Records", "Reports", "Reference"]) expect(within(nav).getByText(g)).toBeInTheDocument();
  expect(within(nav).queryByText("Data")).toBeNull();
});

test.each([
  ["Inbox", "/inbox"],
  ["Adapters", "/adapters"],
  ["Exports", "/exports"],
])("%s opens a working screen from the sidebar", async (label, path) => {
  const { router } = renderApp();
  const nav = screen.getByRole("navigation", { name: "Main" });
  await userEvent.click(within(nav).getByRole("link", { name: label }));
  expect(await screen.findByRole("heading", { name: label, level: 1 })).toBeInTheDocument();
  expect(router.state.location.pathname).toBe(path);
  expect(within(nav).getByRole("link", { name: label })).toHaveAttribute("aria-current", "page");
  expect(screen.queryByText("Not available yet")).toBeNull();
});

test("an account page keeps Accounts current", async () => {
  renderApp("/accounts/1");
  const nav = screen.getByRole("navigation", { name: "Main" });
  expect(within(nav).getByRole("link", { name: "Accounts" })).toHaveAttribute("aria-current", "page");
});

test("navigating to Tax opens the Tax screen", async () => {
  renderApp();
  const nav = screen.getByRole("navigation", { name: "Main" });
  await userEvent.click(within(nav).getByRole("link", { name: "Tax" }));
  expect(await screen.findByRole("heading", { name: "Tax", level: 1 })).toBeInTheDocument();
  expect(within(nav).getByRole("link", { name: "Tax" })).toHaveAttribute("aria-current", "page");
  expect(await screen.findByText("No taxable activity yet")).toBeInTheDocument();
});

test("navigating to Market data opens its Prices tab", async () => {
  renderApp();
  const nav = screen.getByRole("navigation", { name: "Main" });
  await userEvent.click(within(nav).getByRole("link", { name: "Market data" }));
  expect(await screen.findByRole("heading", { name: "Market data", level: 1 })).toBeInTheDocument();
  expect(within(nav).getByRole("link", { name: "Market data" })).toHaveAttribute("aria-current", "page");
  expect(screen.getByRole("button", { name: "Prices" })).toHaveAttribute("aria-pressed", "true");
});

test("navigating marks the current entry", async () => {
  renderApp();
  const nav = screen.getByRole("navigation", { name: "Main" });
  expect(within(nav).getByRole("link", { name: "Overview" })).toHaveAttribute("aria-current", "page");
  await userEvent.click(within(nav).getByRole("link", { name: "Holdings" }));
  expect(await screen.findByRole("heading", { name: "Holdings", level: 1 })).toBeInTheDocument();
  expect(within(nav).getByRole("link", { name: "Holdings" })).toHaveAttribute("aria-current", "page");
  expect(within(nav).getByRole("link", { name: "Overview" })).not.toHaveAttribute("aria-current");
});

test("layout CSS wraps the content under the sidebar at narrow widths", () => {
  // jsdom has no layout engine; check the rules that produce the stacking.
  const css = readFileSync(resolve(dirname(fileURLToPath(import.meta.url)), "layout.module.css"), "utf8");
  expect(css).toMatch(/\.shell\s*{[^}]*flex-wrap:\s*wrap/);
  expect(css).toMatch(/\.sidebar\s*{[^}]*flex:\s*1 1 var\(--sidebar-width\)/);
  expect(css).toMatch(/\.content\s*{[^}]*flex:\s*999 1 560px[^}]*min-width:\s*0/);
});

test("the Inbox badge counts open items and hides at zero; Imports has none", async () => {
  mockInvoke.mockImplementation(invokeFrom({ ...EMPTY, cmd_inbox_count: 7, cmd_pending_decisions: 2 }) as typeof invoke);
  const { unmount } = renderApp();
  const nav = screen.getByRole("navigation", { name: "Main" });
  const badge = await within(nav).findByLabelText("7 to review");
  expect(within(nav).getByRole("link", { name: /Inbox/ })).toContainElement(badge);
  expect(within(within(nav).getByRole("link", { name: "Imports" })).queryByLabelText(/to review|awaiting/)).toBeNull();
  unmount();
  mockInvoke.mockImplementation(invokeFrom(EMPTY) as typeof invoke);
  renderApp();
  await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("cmd_inbox_count"));
  expect(screen.queryByLabelText(/to review/)).toBeNull();
});

test("native menu navigate event switches screens", async () => {
  const { router } = renderApp();
  await waitFor(() => expect(handlers["menu:navigate"]).toBeDefined());
  handlers["menu:navigate"]!({ payload: "performance" });
  await waitFor(() => expect(router.state.location.pathname).toBe("/performance"));
  handlers["menu:import"]!({ payload: null });
  await waitFor(() => expect(router.state.location.pathname).toBe("/imports"));
  expect(router.state.location.state).toHaveProperty("pick");
});

test("Activity is a working screen, not a placeholder", async () => {
  renderApp("/activity");
  expect(await screen.findByRole("heading", { name: "Activity", level: 1 })).toBeInTheDocument();
  expect(await screen.findByText("No activities yet")).toBeInTheDocument();
  expect(screen.queryByText("Not available yet")).toBeNull();
});

test("Accounts is a working screen, not a placeholder", async () => {
  renderApp("/accounts");
  expect(await screen.findByRole("heading", { name: "Accounts", level: 1 })).toBeInTheDocument();
  expect(await screen.findByText("No accounts yet")).toBeInTheDocument();
  expect(screen.queryByText("Not available yet")).toBeNull();
});

test("Income is a working screen, not a placeholder", async () => {
  renderApp("/income");
  expect(await screen.findByRole("heading", { name: "Income", level: 1 })).toBeInTheDocument();
  expect(screen.queryByText("Not available yet")).toBeNull();
});

test.each([
  ["/", "your accounts"],
  ["/holdings", "your positions"],
  ["/performance", "contributions"],
])("empty database on %s links to Imports", async (path, what) => {
  renderApp(path);
  const status = await screen.findByRole("status");
  expect(status).toHaveTextContent(`Import a statement batch to see ${what}`);
  expect(within(status).getByRole("link", { name: "Go to Imports" })).toHaveAttribute("href", "/imports");
});

test.each(["Assets", "Settings"])("%s is a working screen, not a placeholder", async (title) => {
  renderApp(`/${title.toLowerCase()}`);
  expect(await screen.findByRole("heading", { name: title, level: 1 })).toBeInTheDocument();
  expect(screen.queryByText("Not available yet")).toBeNull();
});

test("sidebar data status: never refreshed, refreshed, and history downloading", async () => {
  const { unmount } = renderApp();
  const nav = screen.getByRole("navigation", { name: "Main" });
  expect(await within(nav).findByText("Prices not refreshed yet")).toBeInTheDocument();
  expect(within(nav).getByRole("link", { name: /Data status/ })).toHaveAttribute("href", "/market-data");
  unmount();

  const running = {
    ...EMPTY.cmd_backfill_status,
    sources: EMPTY.cmd_backfill_status.sources.map((x, i) =>
      i === 0 ? { ...x, state: "running", done: 31, planned: 50 } : i === 1 ? { ...x, state: "waiting", done: 0, planned: 0 } : x,
    ),
  };
  mockInvoke.mockImplementation(
    invokeFrom({ ...EMPTY, cmd_settings: { ...EMPTY.cmd_settings, last_refresh_at: "2026-10-30T10:00:00Z" }, cmd_backfill_status: running }) as typeof invoke,
  );
  renderApp();
  const nav2 = screen.getByRole("navigation", { name: "Main" });
  expect(await within(nav2).findByText("Prices refreshed 30 Oct 2026")).toBeInTheDocument();
  expect(within(nav2).getByText("Downloading history · 62%")).toBeInTheDocument();
  expect(within(nav2).getByRole("progressbar", { name: "History download" })).toHaveAttribute("aria-valuenow", "62");
});

test("sidebar data status shows a running refresh", async () => {
  mockInvoke.mockImplementation(invokeFrom({ ...EMPTY, cmd_settings: { ...EMPTY.cmd_settings, refreshing: true } }) as typeof invoke);
  renderApp();
  const nav = screen.getByRole("navigation", { name: "Main" });
  expect(await within(nav).findByText("Refreshing prices…")).toBeInTheDocument();
});

describe("native menu events", () => {
  const ready = async () => {
    const r = renderApp();
    await waitFor(() => expect(handlers["menu:find"]).toBeDefined());
    return r;
  };

  test("Find opens the command palette", async () => {
    await ready();
    handlers["menu:find"]!({ payload: null });
    expect(await screen.findByRole("dialog", { name: "Search or run a command" })).toBeInTheDocument();
  });

  test("Add activity, Exports and Go items navigate", async () => {
    mockInvoke.mockImplementation(invokeFrom({ ...EMPTY, cmd_view_get: { account_id: 7, period: "ytd", real: false } }) as typeof invoke);
    const { router } = await ready();
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("cmd_view_get"));
    handlers["menu:add-activity"]!({ payload: null });
    await waitFor(() => expect(router.state.location.pathname).toBe("/activity"));
    expect(router.state.location.state).toMatchObject({ add: { accountId: 7 } });
    handlers["menu:exports"]!({ payload: null });
    await waitFor(() => expect(router.state.location.pathname).toBe("/exports"));
    handlers["menu:navigate"]!({ payload: "inbox" });
    await waitFor(() => expect(router.state.location.pathname).toBe("/inbox"));
  });

  test("Back up database opens Exports and asks for a folder", async () => {
    const { open } = await import("@tauri-apps/plugin-dialog");
    vi.mocked(open).mockResolvedValue(null);
    const { router } = await ready();
    handlers["menu:backup"]!({ payload: null });
    await waitFor(() => expect(router.state.location.pathname).toBe("/exports"));
    await waitFor(() => expect(open).toHaveBeenCalledWith(expect.objectContaining({ directory: true })));
  });

  test("Nominal | Real toggles the shared view", async () => {
    await ready();
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("cmd_view_get"));
    handlers["menu:toggle-real"]!({ payload: null });
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("cmd_view_set", { view: { account_id: null, period: "ytd", real: true } }));
  });

  test("Inspector sends the toggle to the window", async () => {
    await ready();
    const seen = vi.fn();
    window.addEventListener("mizan:toggle-inspector", seen);
    handlers["menu:inspector"]!({ payload: null });
    expect(seen).toHaveBeenCalledTimes(1);
    window.removeEventListener("mizan:toggle-inspector", seen);
  });

  test("Refresh prices runs a refresh", async () => {
    mockInvoke.mockImplementation(invokeFrom({ ...EMPTY, cmd_refresh_prices: { started_at: "", finished_at: "", sources: [] } }) as typeof invoke);
    await ready();
    handlers["menu:refresh"]!({ payload: null });
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("cmd_refresh_prices"));
  });

  test("Undo last import asks to confirm undoing the newest import", async () => {
    const batch = (id: number) => ({ id, committed_at: "2026-10-0" + id + "T10:00:00Z", adapter: "ofx", adapter_version: "1", file_sha256: "a".repeat(64), activity_count: id * 10, assertion_count: 0, all_checks_passed: null });
    mockInvoke.mockImplementation(invokeFrom({ ...EMPTY, cmd_import_history: [batch(1), batch(3), batch(2)] }) as typeof invoke);
    const { router } = await ready();
    handlers["menu:undo-import"]!({ payload: null });
    await waitFor(() => expect(router.state.location.pathname).toBe("/imports"));
    expect(await screen.findByText("Remove 30 records?")).toBeInTheDocument();
    expect(mockInvoke).not.toHaveBeenCalledWith("cmd_import_undo", expect.anything());
  });
});
