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

test("lists built-in providers with switches and toggles one off", async () => {
  serve();
  renderApp("/settings");
  const rows = await screen.findAllByTestId("provider-row");
  expect(rows.map((r) => within(r).getAllByRole("cell")[0]!.textContent)).toEqual(["TEFAS", "TCMB", "Yahoo Finance", "Twelve Data", "Eurostat", "BLS", "ONS"]);
  expect(rows[2]).toHaveTextContent("unofficial source");
  expect(rows[3]).toHaveTextContent("the free plan covers US listings");
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

function progress(tefas: Record<string, unknown>, tcmb: Record<string, unknown> = {}) {
  const base = EMPTY.cmd_backfill_status;
  return {
    cmd_backfill_status: {
      ...base,
      sources: [
        { ...base.sources[0], ...tefas },
        { ...base.sources[1], ...tcmb },
      ],
    },
  };
}

test("shows price history progress and the cache location", async () => {
  serve(progress({ state: "running", done: 34, planned: 120 }, { state: "done" }));
  renderApp("/settings");
  const rows = await screen.findAllByTestId("history-row");
  expect(rows[0]).toHaveTextContent("Downloading");
  expect(rows[0]).toHaveTextContent("34 of 120 months");
  expect(rows[1]).toHaveTextContent("Complete");
  expect(screen.getByRole("region", { name: "Price history" })).toHaveTextContent("/Users/a/.mizan/market-history.sqlite");
});

test("shows waiting, failed, disabled and the five-year note", async () => {
  serve(
    progress(
      { state: "failed", error: "TEFAS request failed: connection closed", note: "Fund prices older than five years are not available from TEFAS." },
      { state: "waiting", planned: 12 },
    ),
  );
  renderApp("/settings");
  const rows = await screen.findAllByTestId("history-row");
  expect(rows[0]).toHaveTextContent("Failed");
  expect(rows[0]).toHaveTextContent("connection closed");
  expect(rows[0]).toHaveTextContent("older than five years");
  expect(rows[1]).toHaveTextContent("12 days missing · continues after the next refresh");
});

test("idle and disabled history states", async () => {
  serve(progress({ state: "disabled" }, { state: "idle" }));
  renderApp("/settings");
  const rows = await screen.findAllByTestId("history-row");
  expect(rows[0]).toHaveTextContent("Provider is off");
  expect(rows[1]).toHaveTextContent("Only TRY is used");
});

test("saves and clears the Twelve Data key without ever showing it", async () => {
  serve();
  renderApp("/settings");
  const rows = await screen.findAllByTestId("provider-row");
  expect(within(rows[0]!).queryByLabelText(/API key/)).not.toBeInTheDocument();
  const field = within(rows[3]!).getByLabelText("Twelve Data API key");
  expect(field).toHaveAttribute("type", "password");
  expect(rows[3]).toHaveTextContent("not saved");
  await userEvent.type(field, " td-key-3f9a ");
  await userEvent.click(within(rows[3]!).getByRole("button", { name: "Save key" }));
  expect(invoke).toHaveBeenCalledWith("cmd_set_api_key", { id: "twelvedata", key: "td-key-3f9a" });
});

test("a saved key shows only its masked hint and can be cleared", async () => {
  serve({
    cmd_settings: {
      ...EMPTY.cmd_settings,
      providers: EMPTY.cmd_settings.providers.map((p) => (p.id === "twelvedata" ? { ...p, api_key_hint: "••••3f9a" } : p)),
    },
  });
  renderApp("/settings");
  const rows = await screen.findAllByTestId("provider-row");
  expect(within(rows[3]!).getByTestId("api-key-hint")).toHaveTextContent("••••3f9a");
  await userEvent.click(within(rows[3]!).getByRole("button", { name: "Clear key" }));
  expect(invoke).toHaveBeenCalledWith("cmd_set_api_key", { id: "twelvedata", key: null });
});

test("listed history counts years and waits for an API key", async () => {
  const base = EMPTY.cmd_backfill_status;
  serve({
    cmd_backfill_status: {
      ...base,
      sources: [
        base.sources[0],
        base.sources[1],
        { ...base.sources[2], state: "running", done: 6, planned: 14 },
        { ...base.sources[3], state: "needs_key" },
      ],
    },
  });
  renderApp("/settings");
  const rows = await screen.findAllByTestId("history-row");
  expect(rows[2]).toHaveTextContent("Yahoo Finance");
  expect(rows[2]).toHaveTextContent("6 of 14 years");
  expect(rows[3]).toHaveTextContent("Waiting for an API key");
  expect(rows[3]).toHaveTextContent("Save a Twelve Data API key above");
});

test("idle listed history explains why", async () => {
  serve();
  renderApp("/settings");
  const rows = await screen.findAllByTestId("history-row");
  expect(rows[2]).toHaveTextContent("no asset priced from Yahoo Finance has activities yet");
});

test("inflation index status: cached, provider off, unsupported currency", async () => {
  serve({
    cmd_settings: {
      ...EMPTY.cmd_settings,
      base_currency: "EUR",
      inflation: { label: "Euro-area HICP (Eurostat)", through: "Sep 2026", last_checked: "2026-11-02T10:00:00Z", last_check_failed: false, note: null },
    },
  });
  const { unmount } = renderApp("/settings");
  const status = await screen.findByTestId("inflation-status");
  expect(status).toHaveTextContent("Euro-area HICP (Eurostat) · through Sep 2026");
  expect(status).toHaveTextContent("Last checked 2026-11-02 10:00 UTC");
  unmount();

  serve({
    cmd_settings: {
      ...EMPTY.cmd_settings,
      base_currency: "USD",
      inflation: { label: "US CPI-U (BLS)", through: "Sep 2026", last_checked: null, last_check_failed: false, note: "US CPI-U is not refreshed because BLS is off; cached values are still used." },
    },
  });
  const second = renderApp("/settings");
  expect(await screen.findByTestId("inflation-status")).toHaveTextContent("US CPI-U is not refreshed because BLS is off; cached values are still used.");
  second.unmount();

  serve({
    cmd_settings: {
      ...EMPTY.cmd_settings,
      base_currency: "CHF",
      inflation: { label: null, through: null, last_checked: null, last_check_failed: false, note: "No inflation index for CHF; real returns are not available." },
    },
  });
  renderApp("/settings");
  expect(await screen.findByTestId("inflation-status")).toHaveTextContent("No inflation index for CHF; real returns are not available.");
});
