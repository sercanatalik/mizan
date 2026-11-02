import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { EMPTY, invokeFrom, renderApp } from "../test/render";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
const mockInvoke = vi.mocked(invoke);
vi.mocked(listen);

function serve(extra: Record<string, unknown> = {}) {
  mockInvoke.mockImplementation(
    invokeFrom({
      ...EMPTY,
      cmd_accounts: [{ id: 3, name: "IBKR", hints: [] }],
      cmd_backup_database: "/Users/a/Documents/mizan-backup-20261102-210509.db",
      cmd_export_activities_csv: "/Users/a/Documents/mizan-activities-2026-11-02.csv",
      cmd_export_holdings_csv: "/Users/a/Documents/mizan-holdings-ibkr-2026-11-02.csv",
      cmd_export_income_csv: "/Users/a/Documents/mizan-income-ibkr-2026-01-01-2026-11-02.csv",
      ...extra,
    }) as typeof invoke,
  );
}

beforeEach(() => {
  mockInvoke.mockReset();
  vi.mocked(open).mockResolvedValue("/Users/a/Documents");
  serve();
});

const row = async (title: string) => (await screen.findAllByTestId("export-row")).find((r) => r.textContent?.includes(title))!;

test("backup writes to the chosen folder and shows the path", async () => {
  renderApp("/exports");
  const r = await row("Database backup");
  await userEvent.click(within(r).getByRole("button", { name: "Back up database…" }));
  expect(mockInvoke).toHaveBeenCalledWith("cmd_backup_database", { dir: "/Users/a/Documents" });
  expect(await within(r).findByRole("status")).toHaveTextContent("Saved /Users/a/Documents/mizan-backup-20261102-210509.db");
});

test("a failed backup shows the error", async () => {
  serve({
    cmd_backup_database: () => {
      throw { kind: "io", message: "/Volumes/ro: permission denied" };
    },
  });
  renderApp("/exports");
  const r = await row("Database backup");
  await userEvent.click(within(r).getByRole("button", { name: "Back up database…" }));
  expect(await within(r).findByRole("alert")).toHaveTextContent("permission denied");
});

test("holdings and income use the view bar's scope and period, and say so", async () => {
  serve({ cmd_view_get: { account_id: 3, period: "three_months", real: false } });
  renderApp("/exports");
  const holdings = await row("Holdings CSV");
  await waitFor(() => expect(holdings).toHaveTextContent("IBKR"));
  const income = await row("Income CSV");
  expect(income).toHaveTextContent("IBKR · 3M");
  await userEvent.click(within(holdings).getByRole("button", { name: "Export…" }));
  expect(mockInvoke).toHaveBeenCalledWith("cmd_export_holdings_csv", { account_id: 3, dir: "/Users/a/Documents" });
  await userEvent.click(within(income).getByRole("button", { name: "Export…" }));
  expect(mockInvoke).toHaveBeenCalledWith("cmd_export_income_csv", { period: "three_months", account_id: 3, dir: "/Users/a/Documents" });
  await userEvent.click(within(await row("Activities CSV")).getByRole("button", { name: "Export…" }));
  expect(mockInvoke).toHaveBeenCalledWith("cmd_export_activities_csv", { dir: "/Users/a/Documents" });
});

test("tax and market data exports link to their screens", async () => {
  const { router } = renderApp("/exports");
  const other = await screen.findByRole("region", { name: "Other exports" });
  await userEvent.click(within(other).getByRole("link", { name: "Tax report CSV" }));
  await waitFor(() => expect(router.state.location.pathname).toBe("/tax"));
  expect(within(screen.getByRole("navigation", { name: "Main" })).getByRole("link", { name: "Market data" })).toBeInTheDocument();
});
