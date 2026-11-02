import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { EMPTY, invokeFrom, renderApp } from "../test/render";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

beforeEach(() => {
  vi.mocked(invoke).mockReset();
  vi.mocked(listen).mockImplementation((async () => () => {}) as unknown as typeof listen);
});

function serve(extra: Record<string, unknown> = {}) {
  vi.mocked(invoke).mockImplementation(
    invokeFrom({
      ...EMPTY,
      cmd_set_provider_enabled: undefined,
      cmd_set_api_key: undefined,
      cmd_set_base_currency: undefined,
      cmd_set_refresh_on_start: undefined,
      ...extra,
    }) as typeof invoke,
  );
}

test("changes base currency", async () => {
  serve();
  renderApp("/settings");
  await userEvent.selectOptions(await screen.findByRole("combobox", { name: "Base currency" }), "EUR");
  expect(invoke).toHaveBeenCalledWith("cmd_set_base_currency", { currency: "EUR" });
});

test("providers are no longer in Settings; a link points to Market data › Providers", async () => {
  serve();
  const { router } = renderApp("/settings");
  const card = await screen.findByRole("region", { name: "Price providers" });
  expect(screen.queryAllByTestId("provider-row")).toHaveLength(0);
  expect(screen.queryByRole("checkbox", { name: /Refresh prices when the app starts/ })).toBeNull();
  await userEvent.click(within(card).getByRole("link", { name: "Market data › Providers" }));
  await waitFor(() => expect(router.state.location.pathname).toBe("/market-data"));
  expect(await screen.findAllByTestId("provider-row")).toHaveLength(8);
});

describe("reset data", () => {
  const portfolio = { accounts: 4, activities: 424, imports: 3, assets: 9, cache_values: null, cache_path: null };
  const everything = { ...portfolio, cache_values: 52000, cache_path: "/Users/a/.mizan/market-history.sqlite" };

  test("preview shows what each scope clears, with the shared-cache warning for everything", async () => {
    serve({ cmd_reset_preview: (a: unknown) => ((a as { scope: string }).scope === "everything" ? everything : portfolio) });
    renderApp("/settings");
    const card = await screen.findByRole("region", { name: "Reset data" });
    expect(await within(card).findByTestId("reset-preview")).toHaveTextContent("4 accounts, 424 activities, 3 imports and 9 assets");
    expect(within(card).getByTestId("reset-preview")).not.toHaveTextContent("Other Mizan databases");
    await userEvent.click(within(card).getByRole("radio", { name: /Everything/ }));
    await waitFor(() => expect(within(card).getByTestId("reset-preview")).toHaveTextContent("52,000 cached prices"));
    expect(within(card).getByTestId("reset-preview")).toHaveTextContent("Other Mizan databases");
  });

  test("the reset runs only after typing RESET and shows the backups", async () => {
    serve({
      cmd_reset_preview: portfolio,
      cmd_reset_data: { backups: ["/Users/a/Library/Application Support/mizan/mizan-backup-20261102-101500.db"] },
    });
    renderApp("/settings");
    const card = await screen.findByRole("region", { name: "Reset data" });
    const button = within(card).getByRole("button", { name: "Reset portfolio data" });
    const field = within(card).getByRole("textbox", { name: "Type RESET to confirm" });
    await userEvent.type(field, "reset");
    expect(button).toBeDisabled();
    await userEvent.clear(field);
    await userEvent.type(field, "RESET");
    expect(button).toBeEnabled();
    await userEvent.click(button);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("cmd_reset_data", { scope: "portfolio", confirm: "RESET" }));
    expect(await within(card).findByTestId("reset-done")).toHaveTextContent("mizan-backup-20261102-101500.db");
  });

  test("a failed reset shows the error", async () => {
    serve({
      cmd_reset_preview: portfolio,
      cmd_reset_data: () => Promise.reject({ kind: "storage", message: "database error: disk full" }),
    });
    renderApp("/settings");
    const card = await screen.findByRole("region", { name: "Reset data" });
    await userEvent.type(within(card).getByRole("textbox", { name: "Type RESET to confirm" }), "RESET");
    await userEvent.click(within(card).getByRole("button", { name: "Reset portfolio data" }));
    expect(await within(card).findByRole("alert")).toHaveTextContent("disk full");
  });
});
