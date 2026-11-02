import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import type { SeriesDetail } from "../bridge/generated/SeriesDetail";
import type { SeriesPoint } from "../bridge/generated/SeriesPoint";
import type { SeriesRow } from "../bridge/generated/SeriesRow";
import { EMPTY, invokeFrom, renderApp } from "../test/render";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
const mockInvoke = vi.mocked(invoke);
const mockOpen = vi.mocked(open);

beforeEach(() => {
  vi.mocked(listen).mockResolvedValue(() => {});
});

const row = (key: string, extra: Partial<SeriesRow> = {}): SeriesRow => ({
  kind: "price",
  key,
  name: key.split(":")[1] ?? key,
  code: key.split(":")[1] ?? key,
  source: key.split(":")[0] ?? "tcmb",
  currency: "TRY",
  latest: "1.10",
  latest_date: "2026-10-31",
  first_date: "2022-01-03",
  count: 1201,
  stale: false,
  series_gaps: 0,
  held_gaps: 0,
  edits: 0,
  used: true,
  ...extra,
});

const point = (p: string, extra: Partial<SeriesPoint> = {}): SeriesPoint => ({
  period: p,
  value: "39.00",
  currency: "TRY",
  source: "cache",
  fetched_at: "2026-11-01T09:12:00Z",
  original: null,
  tag: null,
  edited_at: null,
  in_use: true,
  ...extra,
});

const prices = {
  rows: [
    row("tefas:AFT", { name: "Ak Portföy Teknoloji", held_gaps: 3, series_gaps: 41 }),
    row("tefas:TTE", { used: false, held_gaps: null, series_gaps: 12 }),
    row("yahoo:VWCE.DE", { currency: "EUR", stale: true, latest: "131.42", latest_date: "2026-10-19" }),
  ],
  total: 3,
  sources: ["tefas", "yahoo"],
  currencies: ["EUR", "TRY"],
};

const usdDetail: SeriesDetail = {
  row: row("USD", { kind: "rate", key: "USD", name: "USD", code: "USD", source: "tcmb", edits: 1, held_gaps: 2, series_gaps: 57 }),
  points: [
    point("2025-06-19"),
    point("2025-06-10", { value: "39.05", source: "edited", tag: "edited", original: "39.00", edited_at: "2026-11-02T10:00:00Z" }),
    point("2025-06-09", { tag: "ignored", in_use: false }),
  ],
  total: 3,
  offset: 0,
  chart: [
    { period: "2025-06-10", value: "39.05", edited: true },
    { period: "2025-06-19", value: "39.00", edited: false },
  ],
  manual_only: false,
};

function serve(extra: Record<string, unknown> = {}) {
  mockInvoke.mockImplementation(
    invokeFrom({
      ...EMPTY,
      cmd_market_series: (a: unknown) => {
        const { kind } = a as { kind: string };
        if (kind === "rate") return { rows: [usdDetail.row], total: 1, sources: ["tcmb"], currencies: ["TRY"] };
        if (kind === "index") return { rows: [row("eurostat:TR:I15", { kind: "index", name: "HICP Türkiye (Eurostat)", currency: null })], total: 1, sources: ["eurostat"], currencies: [] };
        return prices;
      },
      cmd_market_series_detail: usdDetail,
      cmd_market_set_value: null,
      cmd_market_ignore: null,
      cmd_market_revert: null,
      cmd_market_export_csv: "/Users/a/Desktop/mizan-rate-USD.csv",
      ...extra,
    }) as typeof invoke,
  );
}

async function openUsd() {
  renderApp("/market-data");
  await userEvent.click(await screen.findByRole("button", { name: "Rates" }));
  await userEvent.click(await screen.findByRole("button", { name: "Open USD" }));
  return screen.findByRole("region", { name: "Series detail" });
}

test("lists every cached series with quality markers", async () => {
  serve();
  renderApp("/market-data");
  const rows = await screen.findAllByTestId("series-row");
  expect(rows).toHaveLength(3);
  expect(rows[0]).toHaveTextContent("Ak Portföy Teknoloji");
  expect(within(rows[0]!).getByTestId("held-gaps")).toHaveTextContent("3");
  expect(within(rows[1]!).getByTestId("held-gaps")).toHaveTextContent("–");
  expect(rows[1]).not.toHaveTextContent("in use");
  expect(within(rows[2]!).getByText("Stale")).toBeInTheDocument();
  expect(screen.getByText(/holidays/)).toBeInTheDocument();
});

test("filters are sent to the backend", async () => {
  serve();
  renderApp("/market-data");
  await screen.findAllByTestId("series-row");
  await userEvent.selectOptions(screen.getByRole("combobox", { name: "Source" }), "tefas");
  await userEvent.click(screen.getByRole("button", { name: "Used by my assets" }));
  await userEvent.click(screen.getByRole("button", { name: "Stale or gaps" }));
  await userEvent.type(screen.getByRole("searchbox", { name: "Search series" }), "aft");
  await waitFor(() =>
    expect(mockInvoke).toHaveBeenCalledWith("cmd_market_series", {
      kind: "price",
      filter: { source: "tefas", currency: null, text: "aft", used: true, edited: false, attention: true },
    }),
  );
});

test("tabs switch between prices, rates and indices", async () => {
  serve();
  renderApp("/market-data");
  await screen.findAllByTestId("series-row");
  await userEvent.click(screen.getByRole("button", { name: "Indices" }));
  expect(await screen.findByText("HICP Türkiye (Eurostat)")).toBeInTheDocument();
  expect(screen.queryByRole("combobox", { name: "Currency" })).toBeNull();
});

test("series detail shows edited and ignored values", async () => {
  serve();
  const detail = await openUsd();
  expect(detail).toHaveTextContent("Edits stay in this database");
  const gaps = within(detail).getByTestId("gap-counts");
  expect(gaps).toHaveTextContent("Gaps while held: 2");
  expect(gaps).toHaveTextContent("Series gaps: 57");
  const values = within(detail).getAllByTestId("value-row");
  expect(values[1]).toHaveTextContent("39.05");
  expect(values[1]).toHaveTextContent("was 39.00");
  expect(within(values[1]!).getByText("Edited")).toBeInTheDocument();
  expect(within(values[2]!).getByText("Ignored")).toBeInTheDocument();
  expect(within(values[2]!).queryByRole("button", { name: "Ignore" })).toBeNull();
  expect(within(values[0]!).queryByRole("button", { name: "Revert" })).toBeNull();
});

test("edit, ignore and revert a value", async () => {
  serve();
  const detail = await openUsd();
  const values = within(detail).getAllByTestId("value-row");
  await userEvent.click(within(values[0]!).getByRole("button", { name: "Edit" }));
  const input = within(values[0]!).getByRole("textbox", { name: "New value for 2025-06-19" });
  await userEvent.clear(input);
  await userEvent.type(input, "39.45");
  await userEvent.click(within(values[0]!).getByRole("button", { name: "Save" }));
  await waitFor(() =>
    expect(mockInvoke).toHaveBeenCalledWith("cmd_market_set_value", { kind: "rate", key: "USD", period: "2025-06-19", value: "39.45", currency: null }),
  );
  await userEvent.click(within(values[0]!).getByRole("button", { name: "Ignore" }));
  await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("cmd_market_ignore", { kind: "rate", key: "USD", period: "2025-06-19" }));
  await userEvent.click(within(values[1]!).getByRole("button", { name: "Revert" }));
  await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("cmd_market_revert", { kind: "rate", key: "USD", period: "2025-06-10" }));
});

test("add a missing value", async () => {
  serve();
  const detail = await openUsd();
  const form = within(detail).getByRole("form", { name: "Add value" });
  await userEvent.type(within(form).getByLabelText("Date"), "2025-04-01");
  await userEvent.type(within(form).getByRole("textbox", { name: "Value" }), "38.10");
  await userEvent.click(within(form).getByRole("button", { name: "Add value" }));
  await waitFor(() =>
    expect(mockInvoke).toHaveBeenCalledWith("cmd_market_set_value", { kind: "rate", key: "USD", period: "2025-04-01", value: "38.10", currency: null }),
  );
});

test("export writes a CSV only when a folder is chosen", async () => {
  serve();
  const detail = await openUsd();
  mockOpen.mockResolvedValueOnce(null);
  await userEvent.click(within(detail).getByRole("button", { name: "Export CSV" }));
  expect(mockInvoke).not.toHaveBeenCalledWith("cmd_market_export_csv", expect.anything());
  mockOpen.mockResolvedValueOnce("/Users/a/Desktop");
  await userEvent.click(within(detail).getByRole("button", { name: "Export CSV" }));
  await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("cmd_market_export_csv", { kind: "rate", key: "USD", dir: "/Users/a/Desktop" }));
  expect(await screen.findByTestId("export-done")).toHaveTextContent("mizan-rate-USD.csv");
});

test("an empty cache points to Assets", async () => {
  serve({ cmd_market_series: { rows: [], total: 0, sources: [], currencies: [] } });
  renderApp("/market-data");
  expect(await screen.findByText("No market data yet")).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Go to Assets" })).toHaveAttribute("href", "/assets");
});
