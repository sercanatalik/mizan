import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import type { ImportView } from "../bridge/generated/ImportView";
import { reviewView } from "../test/fixtures";
import { EMPTY, invokeFrom, renderApp } from "../test/render";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
const mockInvoke = vi.mocked(invoke);
const mockOpen = vi.mocked(open);
const mockSave = vi.mocked(save);

function respond(extra: Record<string, unknown>) {
  mockInvoke.mockImplementation(invokeFrom({ ...EMPTY, ...extra }) as typeof invoke);
}

beforeEach(() => {
  mockInvoke.mockReset();
  mockOpen.mockReset();
  mockSave.mockReset();
  vi.mocked(listen).mockImplementation((async () => () => {}) as unknown as typeof listen);
});

const stat = (label: string) => within(screen.getByTestId(`stat-${label}`));

// ---- 9.1 ----

const review = (view: ImportView) => ({ type: "review", view });

test("opening a valid batch shows source and summary counts", async () => {
  mockOpen.mockResolvedValue("/tmp/degiro-2026Q3.json");
  respond({ cmd_import_open: () => review(reviewView()) });
  renderApp("/imports");
  expect(await screen.findByText("No import in review")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Open file" }));

  expect(mockInvoke).toHaveBeenCalledWith("cmd_import_open", { path: "/tmp/degiro-2026Q3.json" });
  expect(await screen.findByText("Review · degiro-2026Q3.json · batch file")).toBeInTheDocument();
  expect(screen.getByTestId("adapter")).toHaveTextContent("batch file");
  expect(screen.getByText("2026-07-01 – 2026-09-30")).toBeInTheDocument();
  expect(stat("New").getByText("38")).toBeInTheDocument();
  expect(stat("Duplicates").getByText("101")).toBeInTheDocument();
  expect(stat("Need your decision").getByText("2")).toBeInTheDocument();
});

test("unsupported version shows the reason and opens no review", async () => {
  mockOpen.mockResolvedValue("/tmp/v9.json");
  respond({
    cmd_import_open: () => {
      throw { kind: "batch", message: 'unsupported schema version "mizan.import/9"' };
    },
  });
  renderApp("/imports");
  await userEvent.click(await screen.findByRole("button", { name: "Open file" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("mizan.import/9");
  expect(screen.getByText("No import in review")).toBeInTheDocument();
});

test("cancelling the picker does nothing", async () => {
  mockOpen.mockResolvedValue(null);
  respond({});
  renderApp("/imports");
  await userEvent.click(await screen.findByRole("button", { name: "Open file" }));
  expect(mockInvoke).not.toHaveBeenCalledWith("cmd_import_open", expect.anything());
});

// ---- adapters ----

const ibkr = { id: "ibkr-flex", name: "Interactive Brokers Flex Query", version: "1.0.0", score: 95, user: false };
const mine = { id: "my-broker", name: "My broker", version: "1.0.0", score: 95, user: true };
const fromAdapter = (adapter: typeof ibkr, matches = [adapter]): ImportView => {
  const v = reviewView();
  return { ...v, file_name: "flex.xml", source: { ...v.source, adapter, matches, adapters: [`${adapter.id} ${adapter.version}`] } };
};

test("a statement opens through its adapter and the picker accepts statement files", async () => {
  mockOpen.mockResolvedValue("/tmp/flex.xml");
  respond({ cmd_import_open: () => review(fromAdapter(ibkr)) });
  renderApp("/imports");
  await userEvent.click(await screen.findByRole("button", { name: "Open file" }));
  const filters = (mockOpen.mock.calls[0]![0] as { filters: { extensions: string[] }[] }).filters[0]!.extensions;
  expect(filters).toEqual(expect.arrayContaining(["csv", "xlsx", "pdf", "ofx", "qfx", "xml", "json"]));
  expect(await screen.findByText("Review · flex.xml · ibkr-flex 1.0.0")).toBeInTheDocument();
  expect(screen.getByTestId("adapter")).toHaveTextContent("ibkr-flex 1.0.0");
  expect(screen.queryByLabelText("Read with")).not.toBeInTheDocument();
});

test("other matching adapters can be chosen", async () => {
  respond({
    cmd_import_current: fromAdapter(mine, [mine, ibkr]),
    cmd_import_choose_adapter: (a: unknown) => {
      expect(a).toEqual({ id: "ibkr-flex" });
      return fromAdapter(ibkr, [mine, ibkr]);
    },
  });
  renderApp("/imports");
  const select = await screen.findByLabelText("Read with");
  expect(within(select).getByRole("option", { name: /My broker \(my-broker 1.0.0\) · yours/ })).toBeInTheDocument();
  await userEvent.selectOptions(select, "ibkr-flex");
  await waitFor(() => expect(screen.getByTestId("adapter")).toHaveTextContent("ibkr-flex 1.0.0"));
});

test("an unrecognized CSV offers the column mapper; a PDF does not", async () => {
  mockOpen.mockResolvedValueOnce("/tmp/mystery.csv").mockResolvedValueOnce("/tmp/scan.pdf");
  respond({
    cmd_import_open: (a: unknown) => {
      const path = (a as { path: string }).path;
      return { type: "unrecognized", file_name: path.split("/").pop(), file_kind: path.endsWith(".pdf") ? "pdf" : "csv" };
    },
    cmd_mapper_inspect: INSPECT,
    cmd_mapper_preview: PREVIEW,
  });
  renderApp("/imports");
  await userEvent.click(await screen.findByRole("button", { name: "Open file" }));
  const card = await screen.findByRole("region", { name: "Not recognized" });
  expect(card).toHaveTextContent("mystery.csv is not recognized by any adapter");
  expect(screen.getByText("No import in review")).toBeInTheDocument();

  await userEvent.click(screen.getByRole("button", { name: "Open file" }));
  await waitFor(() => expect(screen.getByRole("region", { name: "Not recognized" })).toHaveTextContent("scan.pdf"));
  expect(screen.queryByRole("button", { name: "Open column mapper" })).not.toBeInTheDocument();
});

// ---- column mapper ----

const INSPECT = {
  file_name: "bank.csv",
  file_kind: "csv",
  options: { delimiter: ";", encoding: "windows-1254", skip_rows: 0, sheet: null },
  sheets: [],
  header: ["İşlem Tarihi", "Açıklama", "Tutar"],
  rows: [
    { row: 2, cells: ["02.10.2026", "Maaş", "25.000,00"] },
    { row: 3, cells: ["05.10.2026", "Kira", "-12.500,00"] },
  ],
  total_rows: 2,
};

const PREVIEW = (a: unknown) => {
  const m = (a as { mapping: { input: { date_format: string } } }).mapping;
  if (m.input.date_format !== "%d.%m.%Y") {
    return { records: [], total_records: 0, diagnostics: [{ severity: "warning", message: `date: "02.10.2026" does not match date format ${m.input.date_format}`, locator: "row 2" }], error: null };
  }
  return {
    records: [
      { locator: "row 2", date: "2026-10-02", kind: "deposit", account: "bank", instrument: null, quantity: null, amount: "25000.00", currency: "TRY" },
      { locator: "row 3", date: "2026-10-05", kind: "withdrawal", account: "bank", instrument: null, quantity: null, amount: "12500.00", currency: "TRY" },
    ],
    total_records: 2,
    diagnostics: [],
    error: null,
  };
};

async function openMapper(extra: Record<string, unknown> = {}) {
  mockOpen.mockResolvedValue("/tmp/bank.csv");
  respond({
    cmd_import_open: { type: "unrecognized", file_name: "bank.csv", file_kind: "csv" },
    cmd_mapper_inspect: INSPECT,
    cmd_mapper_preview: PREVIEW,
    ...extra,
  });
  renderApp("/imports");
  await userEvent.click(await screen.findByRole("button", { name: "Open file" }));
  await userEvent.click(await screen.findByRole("button", { name: "Open column mapper" }));
  expect(await screen.findByText("Column mapper · bank.csv")).toBeInTheDocument();
}

const lastPreviewMapping = () => {
  const calls = mockInvoke.mock.calls.filter(([c]) => c === "cmd_mapper_preview");
  return (calls.at(-1)![1] as { mapping: Record<string, any> }).mapping; // eslint-disable-line @typescript-eslint/no-explicit-any
};

test("the mapper guesses a mapping from the header and previews it", async () => {
  await openMapper();
  expect(mockInvoke).toHaveBeenCalledWith("cmd_mapper_inspect", { path: "/tmp/bank.csv", options: null });
  expect(screen.getByLabelText("Date (required)")).toHaveValue("col:İşlem Tarihi");
  expect(screen.getByLabelText("Amount (net cash moved)")).toHaveValue("col:Tutar");
  expect(screen.getByLabelText("Description")).toHaveValue("col:Açıklama");
  expect(screen.getAllByTestId("rule")).toHaveLength(2);
  const rows = await screen.findAllByTestId("preview-row");
  expect(rows[0]).toHaveTextContent("2026-10-02deposit");
  const m = lastPreviewMapping();
  expect(m.input).toMatchObject({ delimiter: ";", encoding: "windows-1254", decimal: ",", thousands: ".", date_format: "%d.%m.%Y" });
  expect(m.fields.net_cash).toEqual({ amount: { column: "Tutar", transform: ["abs"] }, currency: { const: "TRY" } });
  expect(m.rules[0]).toEqual({ column: "Tutar", matches: "^-", kind: "withdrawal" });
  expect(m.detect.required_columns).toEqual(["İşlem Tarihi", "Açıklama", "Tutar"]);
});

test("editing the mapping updates the preview and shows bad dates", async () => {
  await openMapper();
  await screen.findAllByTestId("preview-row");
  await userEvent.selectOptions(screen.getByLabelText("Date format"), "%Y-%m-%d");
  const diag = await screen.findByTestId("preview-diagnostic");
  expect(diag).toHaveTextContent("row 2");
  expect(diag).toHaveTextContent("does not match date format %Y-%m-%d");
  expect(screen.queryAllByTestId("preview-row")).toHaveLength(0);

  // Rules: add one, change its kind, move it first.
  await userEvent.click(screen.getByRole("button", { name: "Add rule" }));
  await userEvent.selectOptions(screen.getByLabelText("Rule 3 column"), "Açıklama");
  await userEvent.type(screen.getByLabelText("Rule 3 value"), "Kira");
  await userEvent.selectOptions(screen.getByLabelText("Rule 3 kind"), "skip");
  await userEvent.click(screen.getByRole("button", { name: "Move rule 3 up" }));
  await userEvent.click(screen.getByRole("button", { name: "Move rule 2 up" }));
  await waitFor(() => expect(lastPreviewMapping().rules[0]).toEqual({ column: "Açıklama", equals: "Kira", kind: "skip" }));
  await userEvent.click(screen.getByRole("button", { name: "Remove rule 1" }));
  await waitFor(() => expect(lastPreviewMapping().rules).toHaveLength(2));
});

test("saving names the mapping, then export and review use it", async () => {
  mockSave.mockResolvedValue("/tmp/out/my-bank-csv.toml");
  let opened = 0;
  await openMapper({
    cmd_mapper_save: (a: unknown) => {
      const m = (a as { mapping: { adapter: { id: string; name: string } } }).mapping;
      return { id: m.adapter.id, name: m.adapter.name, path: `/data/adapters/${m.adapter.id}.toml` };
    },
    cmd_mapper_export: undefined,
    cmd_import_open: () =>
      opened++ === 0 ? { type: "unrecognized", file_name: "bank.csv", file_kind: "csv" } : review(fromAdapter({ ...mine, id: "my-bank-csv", name: "My Bank CSV" })),
  });
  await userEvent.type(screen.getByLabelText("Mapping name"), "My Bank CSV");
  await userEvent.click(screen.getByRole("button", { name: "Save mapping" }));
  const saved = mockInvoke.mock.calls.find(([c]) => c === "cmd_mapper_save")![1] as { mapping: { adapter: Record<string, unknown> } };
  expect(saved.mapping.adapter).toEqual({ id: "my-bank-csv", version: "1.0.0", name: "My Bank CSV", type: "mapping", files: ["csv"] });
  expect(await screen.findByText(/Saved “My Bank CSV”/)).toBeInTheDocument();

  await userEvent.click(screen.getByRole("button", { name: "Export…" }));
  await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("cmd_mapper_export", { id: "my-bank-csv", dest: "/tmp/out/my-bank-csv.toml" }));

  await userEvent.click(screen.getByRole("button", { name: "Review this file" }));
  expect(await screen.findByText("Review · flex.xml · my-bank-csv 1.0.0")).toBeInTheDocument();
  expect(mockInvoke).toHaveBeenCalledWith("cmd_import_open", { path: "/tmp/bank.csv" });
});

// ---- 9.2 ----

test("failed balance check is shown first with statement and computed values", async () => {
  respond({ cmd_import_current: reviewView() });
  renderApp("/imports");
  const checks = await screen.findByRole("region", { name: "Balance checks" });
  expect(within(checks).getByText("1 of 1 failed")).toBeInTheDocument();
  const lines = within(checks).getAllByTestId("check-line");
  expect(within(lines[1]!).getByText("isin:NL0010273215")).toBeInTheDocument();
  expect(within(lines[1]!).getByText("3")).toBeInTheDocument();
  expect(within(lines[1]!).getByText("1")).toBeInTheDocument();
  expect(within(checks).getAllByRole("img", { name: "Failed" })).toHaveLength(2);
  // Placed before the records list.
  const records = screen.getByRole("heading", { name: "Records" });
  expect(checks.compareDocumentPosition(records) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
});

test("filters list the right rows with counts", async () => {
  respond({ cmd_import_current: reviewView() });
  renderApp("/imports");
  const group = await screen.findByRole("group", { name: "Filter records" });
  expect(within(group).getByRole("button", { name: /Needs attention/ })).toHaveAttribute("aria-pressed", "true");
  const rows = () => screen.getAllByTestId("record-row").map((r) => r.textContent ?? "");

  // Needs attention: the undecided buy, the figure warning, the diagnostic.
  expect(rows()).toHaveLength(3);
  expect(rows().join()).toContain("row 12");
  expect(rows().join()).toContain("1712.10");
  expect(rows().join()).toContain("row 87");

  await userEvent.click(within(group).getByRole("button", { name: /^New/ }));
  expect(rows()).toHaveLength(2);
  await userEvent.click(within(group).getByRole("button", { name: /Duplicates/ }));
  expect(rows()).toHaveLength(1);
  expect(rows()[0]).toContain("row 20");
  await userEvent.click(within(group).getByRole("button", { name: /^All/ }));
  expect(rows()).toHaveLength(5);
  expect(within(group).getByRole("button", { name: /^All/ })).toHaveTextContent("5");
});

// ---- 9.3 ----

test("a decision updates counts and balance checks; undo reverts", async () => {
  respond({
    cmd_import_current: reviewView(),
    cmd_import_decide: (a: unknown) => reviewView((a as { decision: "new" | null }).decision),
  });
  renderApp("/imports");
  await userEvent.click(await screen.findByRole("button", { name: "Second trade" }));
  expect(mockInvoke).toHaveBeenCalledWith("cmd_import_decide", { index: 0, decision: "new" });
  await waitFor(() => expect(stat("New").getByText("39")).toBeInTheDocument());
  expect(within(screen.getByRole("region", { name: "Balance checks" })).getByText("All 1 match")).toBeInTheDocument();

  await userEvent.click(screen.getByRole("group", { name: "Filter records" }).querySelector("button:last-child")!);
  await userEvent.click(screen.getByRole("button", { name: "Undo choice" }));
  expect(mockInvoke).toHaveBeenCalledWith("cmd_import_decide", { index: 0, decision: null });
  await waitFor(() => expect(stat("New").getByText("38")).toBeInTheDocument());
});

test("unresolved hints offer create and map actions", async () => {
  const unresolved = reviewView(null, {
    unresolved: [
      { type: "unresolved_account", hint: { hint: "U1234567", name_hint: "IBKR" } },
      { type: "unresolved_instrument", key: "isin:DE0007164600", hint: { name: "SAP SE", isin: "DE0007164600" } as never },
    ],
    summary: { ...reviewView().report.summary, unresolved: 2 },
  });
  respond({
    cmd_import_current: unresolved,
    cmd_accounts: [{ id: 7, name: "Degiro", hints: ["DEG-4417"] }],
    // Keep the items unresolved so both actions can be exercised on the same card.
    cmd_import_create_account: () => unresolved,
    cmd_import_map_account: () => unresolved,
    cmd_import_create_asset: () => unresolved,
  });
  renderApp("/imports");
  const card = (await screen.findByRole("heading", { name: "Unresolved" })).closest("section")!;
  await userEvent.click(within(card).getByRole("button", { name: "Create asset" }));
  expect(mockInvoke).toHaveBeenCalledWith("cmd_import_create_asset", { key: "isin:DE0007164600" });

  await userEvent.selectOptions(within(card).getByLabelText("Existing account for U1234567"), "7");
  await userEvent.click(within(card).getByRole("button", { name: "Map" }));
  expect(mockInvoke).toHaveBeenCalledWith("cmd_import_map_account", { hint: "U1234567", account_id: 7 });
});

// ---- 9.4 ----

test("commit is disabled while hints are unresolved", async () => {
  respond({ cmd_import_current: reviewView(null, { summary: { ...reviewView().report.summary, unresolved: 1 } }) });
  renderApp("/imports");
  const bar = await screen.findByRole("region", { name: "Commit" });
  expect(within(bar).getByRole("button", { name: "Commit 38" })).toBeDisabled();
  expect(bar).toHaveTextContent("Resolve the accounts and instruments above before committing");
});

test("commit bar notes undecided and failing checks, then commits", async () => {
  respond({ cmd_import_current: reviewView(), cmd_import_commit: { batch_id: 9, committed: 38, skipped_undecided: 2 } });
  renderApp("/imports");
  const bar = await screen.findByRole("region", { name: "Commit" });
  expect(bar).toHaveTextContent("Commit 38 new records");
  expect(bar).toHaveTextContent("2 undecided records will be skipped");
  expect(bar).toHaveTextContent("1 balance check still failing");
  await userEvent.click(within(bar).getByRole("button", { name: "Commit 38" }));
  expect(mockInvoke).toHaveBeenCalledWith("cmd_import_commit");
  expect(await screen.findByText("No import in review")).toBeInTheDocument();
});

test("history lists imports and undo needs confirmation", async () => {
  respond({
    cmd_import_history: [
      { id: 5, committed_at: "2026-11-01T18:00:00Z", adapter: "degiro-csv", adapter_version: "0.4.0", file_sha256: "b".repeat(64), activity_count: 38, assertion_count: 1, all_checks_passed: true },
    ],
    cmd_import_undo: undefined,
  });
  renderApp("/imports");
  const row = await screen.findByTestId("history-row");
  expect(row).toHaveTextContent("2026-11-01");
  expect(row).toHaveTextContent("degiro-csv 0.4.0");
  expect(row).toHaveTextContent("38 added · checks pass");
  await userEvent.click(within(row).getByRole("button", { name: "Undo" }));
  expect(mockInvoke).not.toHaveBeenCalledWith("cmd_import_undo", expect.anything());
  expect(row).toHaveTextContent("Remove 38 records?");
  await userEvent.click(within(row).getByRole("button", { name: "Confirm undo" }));
  expect(mockInvoke).toHaveBeenCalledWith("cmd_import_undo", { batch_id: 5 });
});
