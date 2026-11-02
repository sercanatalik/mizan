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
    .map((a) => a.textContent)
    .filter((t) => t !== "mizan");
  expect(labels).toEqual(["Overview", "Holdings", "Activity", "Performance", "Income", "Tax", "Imports", "Accounts", "Assets", "Settings"]);
  expect(within(nav).queryByText(/spending|budget/i)).toBeNull();
  expect(within(nav).getByText("Portfolio")).toBeInTheDocument();
  expect(within(nav).getByText("Data")).toBeInTheDocument();
});

test("navigating to Tax opens the Tax screen", async () => {
  renderApp();
  const nav = screen.getByRole("navigation", { name: "Main" });
  await userEvent.click(within(nav).getByRole("link", { name: "Tax" }));
  expect(await screen.findByRole("heading", { name: "Tax", level: 1 })).toBeInTheDocument();
  expect(within(nav).getByRole("link", { name: "Tax" })).toHaveAttribute("aria-current", "page");
  expect(await screen.findByText("No taxable activity yet")).toBeInTheDocument();
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

test("imports badge shows pending decisions and hides at zero", async () => {
  mockInvoke.mockImplementation(invokeFrom({ ...EMPTY, cmd_pending_decisions: 2 }) as typeof invoke);
  const { unmount } = renderApp();
  expect(await screen.findByLabelText("2 awaiting decision")).toHaveTextContent("2");
  unmount();
  mockInvoke.mockImplementation(invokeFrom(EMPTY) as typeof invoke);
  renderApp();
  await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("cmd_pending_decisions"));
  expect(screen.queryByLabelText(/awaiting decision/)).toBeNull();
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

test.each(["Activity", "Income", "Accounts"])("%s is a placeholder without figures", async (title) => {
  renderApp(`/${title.toLowerCase()}`);
  expect(await screen.findByRole("heading", { name: title, level: 1 })).toBeInTheDocument();
  const status = screen.getByRole("status");
  expect(status).toHaveTextContent("Not available yet");
  expect(status.textContent).not.toMatch(/\d/);
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
