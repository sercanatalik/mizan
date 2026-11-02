import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { ActivityDetail } from "../bridge/generated/ActivityDetail";
import type { ActivityPage } from "../bridge/generated/ActivityPage";
import type { ActivityRow } from "../bridge/generated/ActivityRow";
import { EMPTY, invokeFrom, renderApp } from "../test/render";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
const mockInvoke = vi.mocked(invoke);

beforeEach(() => {
  vi.mocked(listen).mockResolvedValue(() => {});
});

const row = (id: number, extra: Partial<ActivityRow> = {}): ActivityRow => ({
  id,
  date: "2025-01-03",
  account_id: 1,
  account: "IBKR",
  kind: "buy",
  instrument: "AAPL · Apple Inc.",
  symbol: "AAPL",
  isin: "US0378331005",
  quantity: "10",
  price: { amount: "150", currency: "USD" },
  net_cash: { amount: "1501", currency: "USD" },
  charges: [{ amount: "1", currency: "USD" }],
  source: { manual: false, adapter: "ibkr-flex", adapter_version: "1.0.0", file: "abcdef012345", locator: "row 12", batch_id: 7, added_at: null, amended: false, edited: false, edited_at: null },
  editable: true,
  deletable: false,
  ...extra,
});

const pageOf = (rows: ActivityRow[], total = rows.length): ActivityPage => ({
  rows,
  total,
  offset: 0,
  totals: [{ currency: "USD", cash_in: "1000.00", cash_out: "600.00", net: "400.00" }],
  has_any: true,
});

const manualRow = row(9, {
  date: "2024-03-04",
  account: "Degiro",
  instrument: "VWCE",
  quantity: "5",
  price: { amount: "110.2", currency: "EUR" },
  net_cash: { amount: "553", currency: "EUR" },
  charges: [{ amount: "2", currency: "EUR" }],
  source: { manual: true, adapter: "manual", adapter_version: "0.1.0", file: "0123456789ab", locator: "manual entry", batch_id: 8, added_at: "2026-11-02T10:00:00Z", amended: false, edited: false, edited_at: null },
  editable: true,
  deletable: true,
});

const detailOf = (r: ActivityRow, extra: Partial<ActivityDetail> = {}): ActivityDetail => ({
  row: r,
  settle_date: null,
  gross: null,
  charges: [{ kind: "commission", amount: "1", currency: "USD" }],
  fx: null,
  legs: [],
  split: null,
  cost_basis: null,
  acquisition_date: null,
  external_id: null,
  links: [],
  memo: null,
  file_sha256: "abcdef0123456789".padEnd(64, "0"),
  import: r.source.manual ? null : { batch_id: 7, committed_at: "2026-10-01T09:00:00Z", adapter: "ibkr-flex" },
  amended_at: r.source.amended ? "2026-11-03T09:00:00Z" : null,
  form: {
    kind: r.kind,
    date: r.date,
    account_id: r.account_id,
    new_account: null,
    asset_id: r.source.manual ? 5 : 4,
    new_asset: null,
    currency: r.price?.currency ?? "USD",
    quantity: r.quantity,
    price: r.price?.amount ?? null,
    net_cash: null,
    gross: null,
    commission: r.charges[0]?.amount ?? null,
    withholding: null,
    cost_basis: null,
    acquisition_date: null,
    split_new: null,
    split_old: null,
    memo: null,
  },
  ...extra,
});

const options = {
  accounts: [
    { id: 1, label: "IBKR", currency: null },
    { id: 2, label: "Degiro", currency: null },
  ],
  assets: [
    { id: 4, label: "AAPL · Apple Inc.", currency: "USD" },
    { id: 5, label: "VWCE", currency: "EUR" },
  ],
  kinds: ["buy", "deposit", "dividend"],
  manual_kinds: EMPTY.cmd_activity_options.manual_kinds,
};

const ok = { errors: [], warnings: [], duplicate: null };

function serve(extra: Record<string, unknown> = {}) {
  mockInvoke.mockImplementation(
    invokeFrom({
      ...EMPTY,
      cmd_activities: pageOf([row(1), manualRow]),
      cmd_activity_options: options,
      cmd_activity_detail: (a: unknown) => ((a as { id: number }).id === 9 ? detailOf(manualRow) : detailOf(row(1))),
      cmd_activity_check: ok,
      cmd_activity_save: { saved_id: 9, check: ok, needs_confirmation: false },
      cmd_activity_delete: null,
      ...extra,
    }) as typeof invoke,
  );
}

test("lists activities with source, totals and count", async () => {
  serve();
  renderApp("/activity");
  const rows = await screen.findAllByTestId("activity-row");
  expect(rows).toHaveLength(2);
  expect(rows[0]).toHaveTextContent("3 Jan 2025");
  expect(rows[0]).toHaveTextContent("AAPL · Apple Inc.");
  expect(rows[0]).toHaveTextContent("150 USD");
  expect(rows[0]).toHaveTextContent("1 USD");
  expect(rows[0]).toHaveTextContent("ibkr-flex, row 12");
  expect(rows[1]).toHaveTextContent("manual");
  expect(screen.getByTestId("totals")).toHaveTextContent("USD: in 1,000.00 · out 600.00 · net 400.00");
  expect(screen.getByTestId("match-count")).toHaveTextContent("2 activities");
});

test("filters are sent to the backend", async () => {
  serve();
  renderApp("/activity");
  await screen.findAllByTestId("activity-row");
  await userEvent.selectOptions(screen.getByRole("combobox", { name: "Account" }), "1");
  await userEvent.selectOptions(screen.getByRole("combobox", { name: "Kind" }), "dividend");
  await userEvent.type(screen.getByRole("searchbox", { name: "Instrument" }), "us0378");
  await userEvent.click(screen.getByRole("button", { name: "Manual" }));
  await waitFor(() =>
    expect(mockInvoke).toHaveBeenCalledWith("cmd_activities", {
      query: { accounts: [1], kinds: ["dividend"], from: null, to: null, source: "manual", text: "us0378" },
      offset: 0,
    }),
  );
});

test("a link with ?account= opens filtered to that account", async () => {
  serve();
  renderApp("/activity?account=2");
  await screen.findAllByTestId("activity-row");
  expect(mockInvoke).toHaveBeenCalledWith("cmd_activities", {
    query: { accounts: [2], kinds: [], from: null, to: null, source: null, text: null },
    offset: 0,
  });
  expect(screen.getByRole("combobox", { name: "Account" })).toHaveValue("2");
});

test("nothing matching offers to clear the filters", async () => {
  serve({ cmd_activities: (a: unknown) => ((a as { query: { text: string | null } }).query.text ? { ...pageOf([]), total: 0, totals: [] } : pageOf([row(1)])) });
  renderApp("/activity");
  await screen.findAllByTestId("activity-row");
  await userEvent.type(screen.getByRole("searchbox", { name: "Instrument" }), "zzz");
  expect(await screen.findByText(/No activities match these filters/)).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Clear filters" }));
  expect(await screen.findAllByTestId("activity-row")).toHaveLength(1);
});

test("show more loads the next page", async () => {
  const many = Array.from({ length: 200 }, (_, i) => row(i + 1));
  serve({
    cmd_activities: (a: unknown) =>
      (a as { offset: number }).offset === 0 ? { ...pageOf(many, 250) } : { ...pageOf(Array.from({ length: 50 }, (_, i) => row(i + 201)), 250), offset: 200 },
  });
  renderApp("/activity");
  expect(await screen.findAllByTestId("activity-row")).toHaveLength(200);
  await userEvent.click(screen.getByRole("button", { name: "Show more (50 left)" }));
  await waitFor(() => expect(screen.getAllByTestId("activity-row")).toHaveLength(250));
  expect(screen.queryByRole("button", { name: /Show more/ })).toBeNull();
});

test("an imported activity's detail links to its import and can be amended but not deleted", async () => {
  serve();
  renderApp("/activity");
  await userEvent.click(await screen.findByRole("button", { name: "Open Buy on 3 Jan 2025" }));
  const d = await screen.findByTestId("activity-detail");
  expect(d).toHaveTextContent("Commission");
  expect(d).toHaveTextContent("ibkr-flex 1.0.0 · row 12");
  expect(screen.getByTestId("imported-note")).toHaveTextContent("undo its import");
  expect(screen.getByRole("link", { name: /Open its import/ })).toHaveAttribute("href", "/imports#batch-7");
  expect(screen.getByRole("button", { name: "Edit" })).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Delete…" })).toBeNull();
  expect(screen.queryByRole("button", { name: "Revert to imported values" })).toBeNull();
});

test("adding a manual buy sends the form and shows the saved entry", async () => {
  serve();
  renderApp("/activity");
  await screen.findAllByTestId("activity-row");
  await userEvent.click(screen.getByRole("button", { name: "Add activity" }));
  await userEvent.selectOptions(screen.getByRole("combobox", { name: "Account of activity" }), "2");
  await userEvent.selectOptions(screen.getByRole("combobox", { name: "Asset of activity" }), "5");
  const date = screen.getByLabelText("Date");
  await userEvent.clear(date);
  await userEvent.type(date, "2024-03-04");
  await userEvent.type(screen.getByRole("textbox", { name: "Quantity" }), "5");
  await userEvent.type(screen.getByRole("textbox", { name: "Price" }), "110.20");
  await userEvent.type(screen.getByRole("textbox", { name: "Commission" }), "2.00");
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() =>
    expect(mockInvoke).toHaveBeenCalledWith(
      "cmd_activity_save",
      expect.objectContaining({
        form: expect.objectContaining({ kind: "buy", date: "2024-03-04", account_id: 2, asset_id: 5, currency: "EUR", quantity: "5", price: "110.20", commission: "2.00" }),
        confirm_duplicate: false,
        replace_id: null,
      }),
    ),
  );
  expect(await screen.findByTestId("activity-detail")).toHaveTextContent("manual entry");
});

test("a new account and a new asset can be typed in the form", async () => {
  serve();
  renderApp("/activity");
  await screen.findAllByTestId("activity-row");
  await userEvent.click(screen.getByRole("button", { name: "Add activity" }));
  await userEvent.selectOptions(screen.getByRole("combobox", { name: "Account of activity" }), "new");
  await userEvent.type(screen.getByRole("textbox", { name: "New account name" }), "Old broker");
  await userEvent.selectOptions(screen.getByRole("combobox", { name: "Asset of activity" }), "new");
  await userEvent.type(screen.getByRole("textbox", { name: "New asset symbol" }), "ASML");
  await userEvent.type(screen.getByRole("textbox", { name: "New asset isin" }), "NL0010273215");
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() =>
    expect(mockInvoke).toHaveBeenCalledWith(
      "cmd_activity_save",
      expect.objectContaining({
        form: expect.objectContaining({ account_id: null, new_account: "Old broker", new_asset: expect.objectContaining({ symbol: "ASML", isin: "NL0010273215" }) }),
      }),
    ),
  );
});

test("field errors block saving and are shown on the field", async () => {
  serve({
    cmd_activity_save: { saved_id: null, check: { errors: [{ field: "quantity", message: "a quantity is required" }], warnings: [], duplicate: null }, needs_confirmation: false },
  });
  renderApp("/activity");
  await screen.findAllByTestId("activity-row");
  await userEvent.click(screen.getByRole("button", { name: "Add activity" }));
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("a quantity is required");
  expect(screen.getByRole("region", { name: "Activity form" })).toBeInTheDocument();
});

test("consistency warnings appear while typing", async () => {
  serve({ cmd_activity_check: { errors: [], warnings: ["net cash 1600.00 differs from 1500.00 by 100.00"], duplicate: null } });
  renderApp("/activity");
  await screen.findAllByTestId("activity-row");
  await userEvent.click(screen.getByRole("button", { name: "Add activity" }));
  expect(await screen.findByTestId("form-warning")).toHaveTextContent("differs from");
});

test("a possible duplicate is saved only after confirmation", async () => {
  const dup = { activity_id: 1, date: "2025-01-03", source: "ibkr-flex, row 12" };
  let calls = 0;
  serve({
    cmd_activity_save: (a: unknown) => {
      calls++;
      return (a as { confirm_duplicate: boolean }).confirm_duplicate
        ? { saved_id: 9, check: ok, needs_confirmation: false }
        : { saved_id: null, check: { ...ok, duplicate: dup }, needs_confirmation: true };
    },
  });
  renderApp("/activity");
  await screen.findAllByTestId("activity-row");
  await userEvent.click(screen.getByRole("button", { name: "Add activity" }));
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(await screen.findByTestId("duplicate-warning")).toHaveTextContent("3 Jan 2025 activity from ibkr-flex, row 12");
  await userEvent.click(screen.getByRole("button", { name: "Save anyway" }));
  await waitFor(() => expect(calls).toBe(2));
  expect(mockInvoke).toHaveBeenLastCalledWith("cmd_activity_detail", { id: 9 });
});

test("editing a manual entry replaces it", async () => {
  serve();
  renderApp("/activity");
  await userEvent.click(await screen.findByRole("button", { name: "Open Buy on 4 Mar 2024" }));
  await userEvent.click(await screen.findByRole("button", { name: "Edit" }));
  const price = screen.getByRole("textbox", { name: "Price" });
  expect(price).toHaveValue("110.2");
  expect(screen.getByRole("region", { name: "Activity form" })).toHaveTextContent("Edit activity");
  await userEvent.clear(price);
  await userEvent.type(price, "110.02");
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() =>
    expect(mockInvoke).toHaveBeenCalledWith("cmd_activity_save", expect.objectContaining({ form: expect.objectContaining({ price: "110.02" }), replace_id: 9 })),
  );
});

test("deleting a manual entry asks first", async () => {
  serve();
  renderApp("/activity");
  await userEvent.click(await screen.findByRole("button", { name: "Open Buy on 4 Mar 2024" }));
  await userEvent.click(await screen.findByRole("button", { name: "Delete…" }));
  expect(mockInvoke).not.toHaveBeenCalledWith("cmd_activity_delete", expect.anything());
  await userEvent.click(screen.getByRole("button", { name: "Delete" }));
  await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("cmd_activity_delete", { id: 9 }));
  await waitFor(() => expect(screen.queryByTestId("activity-detail")).toBeNull());
});

test("an empty database links to Imports and offers Add activity", async () => {
  serve({ cmd_activities: { rows: [], total: 0, offset: 0, totals: [], has_any: false } });
  renderApp("/activity");
  expect(await screen.findByText("No activities yet")).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Go to Imports" })).toHaveAttribute("href", "/imports");
  await userEvent.click(screen.getByRole("button", { name: "Add activity" }));
  expect(within(screen.getByRole("region", { name: "Activity form" })).getByRole("combobox", { name: "Kind of activity" })).toHaveValue("buy");
});

test("an untouched form shows no errors until a field is left or Save is pressed", async () => {
  serve({
    cmd_activity_check: { errors: [{ field: "quantity", message: "a quantity is required" }], warnings: [], duplicate: null },
    cmd_activity_save: { saved_id: null, check: { errors: [{ field: "quantity", message: "a quantity is required" }], warnings: [], duplicate: null }, needs_confirmation: false },
  });
  renderApp("/activity");
  await screen.findAllByTestId("activity-row");
  await userEvent.click(screen.getByRole("button", { name: "Add activity" }));
  await new Promise((r) => setTimeout(r, 400));
  expect(screen.queryByRole("alert")).toBeNull();
  await userEvent.click(screen.getByRole("textbox", { name: "Quantity" }));
  await userEvent.tab();
  expect(await screen.findByRole("alert")).toHaveTextContent("a quantity is required");
  expect(screen.getByRole("textbox", { name: "Quantity" })).toHaveAttribute("aria-invalid", "true");
});

test("amending an imported activity keeps it and offers revert", async () => {
  const amendedRow = row(1, { price: { amount: "151", currency: "USD" }, source: { ...row(1).source, amended: true, edited: true, edited_at: "2026-11-03T09:00:00Z" } });
  let amended = false;
  serve({
    cmd_activity_save: () => {
      amended = true;
      return { saved_id: 1, check: ok, needs_confirmation: false };
    },
    cmd_activities: () => pageOf([amended ? amendedRow : row(1), manualRow]),
    cmd_activity_detail: (a: unknown) => ((a as { id: number }).id === 9 ? detailOf(manualRow) : detailOf(amended ? amendedRow : row(1))),
    cmd_activity_revert: null,
  });
  renderApp("/activity");
  await userEvent.click(await screen.findByRole("button", { name: "Open Buy on 3 Jan 2025" }));
  await userEvent.click(await screen.findByRole("button", { name: "Edit" }));
  expect(screen.getByRole("region", { name: "Activity form" })).toHaveTextContent("Amend imported activity");
  const price = screen.getByRole("textbox", { name: "Price" });
  await userEvent.clear(price);
  await userEvent.type(price, "151.00");
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("cmd_activity_save", expect.objectContaining({ replace_id: 1, form: expect.objectContaining({ price: "151.00" }) })));
  expect(await screen.findByTestId("imported-note")).toHaveTextContent("Amended 3 Nov 2026");
  const rows = await screen.findAllByTestId("activity-row");
  expect(within(rows[0]!).getByTestId("edited-tag")).toHaveAccessibleName("Edited 3 Nov 2026");
  expect(within(rows[1]!).queryByTestId("edited-tag")).toBeNull();
  expect(screen.getByTestId("edited-detail")).toHaveTextContent("Edited 3 Nov 2026");
  await userEvent.click(screen.getByRole("button", { name: "Revert to imported values" }));
  await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("cmd_activity_revert", { id: 1 }));
});

test("an imported kind the form does not cover edits only its common fields", async () => {
  const exchange = row(3, { kind: "exchange", quantity: null, price: null, net_cash: null, charges: [] });
  serve({
    cmd_activities: pageOf([exchange]),
    cmd_activity_detail: detailOf(exchange),
  });
  renderApp("/activity");
  await userEvent.click(await screen.findByRole("button", { name: "Open Exchange on 3 Jan 2025" }));
  await userEvent.click(await screen.findByRole("button", { name: "Edit" }));
  const form = screen.getByRole("region", { name: "Activity form" });
  expect(within(form).getByRole("combobox", { name: "Kind of activity" })).toBeDisabled();
  expect(within(form).queryByRole("combobox", { name: "Asset of activity" })).toBeNull();
  expect(within(form).getByRole("textbox", { name: "Net cash" })).toBeInTheDocument();
});

test("untouched activities carry no Edited tag; an edited manual entry does", async () => {
  const edited = { ...manualRow, source: { ...manualRow.source, edited: true, edited_at: "2026-11-04T08:00:00Z" } };
  serve({ cmd_activities: pageOf([row(1), edited]) });
  renderApp("/activity");
  const rows = await screen.findAllByTestId("activity-row");
  expect(within(rows[0]!).queryByTestId("edited-tag")).toBeNull();
  expect(within(rows[1]!).getByTestId("edited-tag")).toHaveTextContent("Edited");
  expect(within(rows[1]!).getByTestId("edited-tag")).toHaveAttribute("title", "Edited 4 Nov 2026");
});
