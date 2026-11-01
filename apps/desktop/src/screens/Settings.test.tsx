import { screen, within } from "@testing-library/react";
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
    invokeFrom({ ...EMPTY, cmd_set_provider_enabled: undefined, cmd_set_base_currency: undefined, cmd_set_refresh_on_start: undefined, ...extra }) as typeof invoke,
  );
}

test("lists built-in providers with switches and toggles one off", async () => {
  serve();
  renderApp("/settings");
  const rows = await screen.findAllByTestId("provider-row");
  expect(rows.map((r) => within(r).getAllByRole("cell")[0]!.textContent)).toEqual(["TEFAS", "TCMB"]);
  expect(rows[0]).toHaveTextContent("Turkish investment and pension fund prices");
  const tefas = screen.getByRole("switch", { name: "TEFAS enabled" });
  expect(tefas).toBeChecked();
  await userEvent.click(tefas);
  expect(invoke).toHaveBeenCalledWith("cmd_set_provider_enabled", { id: "tefas", enabled: false });
});

test("changes base currency and refresh-on-start", async () => {
  serve();
  renderApp("/settings");
  await userEvent.selectOptions(await screen.findByRole("combobox", { name: "Base currency" }), "EUR");
  expect(invoke).toHaveBeenCalledWith("cmd_set_base_currency", { currency: "EUR" });
  await userEvent.click(screen.getByRole("checkbox", { name: /Refresh prices when the app starts/ }));
  expect(invoke).toHaveBeenCalledWith("cmd_set_refresh_on_start", { enabled: false });
});

test("shows the last report with a disabled provider", async () => {
  serve({
    cmd_settings: {
      ...EMPTY.cmd_settings,
      last_refresh_at: "2026-11-01T10:00:05Z",
      providers: [
        { ...EMPTY.cmd_settings.providers[0], enabled: false },
        { ...EMPTY.cmd_settings.providers[1], last_success: "2026-11-01T10:00:05Z" },
      ],
      last_report: {
        started_at: "2026-11-01T10:00:00Z",
        finished_at: "2026-11-01T10:00:05Z",
        sources: [
          { id: "tefas", name: "TEFAS", status: "disabled", updated: 0, messages: [] },
          { id: "tcmb", name: "TCMB", status: "ok", updated: 22, messages: ["rates dated 2026-10-30"] },
        ],
      },
    },
  });
  renderApp("/settings");
  const report = await screen.findByRole("list", { name: "Last refresh" });
  expect(report).toHaveTextContent("TEFAS");
  expect(report).toHaveTextContent("Disabled");
  expect(report).toHaveTextContent("22 updated");
  expect(screen.getByRole("switch", { name: "TEFAS enabled" })).not.toBeChecked();
});

test("refresh button triggers a refresh", async () => {
  serve({ cmd_refresh_prices: { started_at: "a", finished_at: "b", sources: [] } });
  renderApp("/settings");
  await userEvent.click(await screen.findByRole("button", { name: "Refresh prices" }));
  expect(invoke).toHaveBeenCalledWith("cmd_refresh_prices");
});
