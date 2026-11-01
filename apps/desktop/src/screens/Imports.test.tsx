import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { reviewView } from "../test/fixtures";
import { EMPTY, invokeFrom, renderApp } from "../test/render";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
const mockInvoke = vi.mocked(invoke);
const mockOpen = vi.mocked(open);

function respond(extra: Record<string, unknown>) {
  mockInvoke.mockImplementation(invokeFrom({ ...EMPTY, ...extra }) as typeof invoke);
}

beforeEach(() => {
  mockInvoke.mockReset();
  mockOpen.mockReset();
  vi.mocked(listen).mockImplementation((async () => () => {}) as unknown as typeof listen);
});

const stat = (label: string) => within(screen.getByTestId(`stat-${label}`));

// ---- 9.1 ----

test("opening a valid batch shows source and summary counts", async () => {
  mockOpen.mockResolvedValue("/tmp/degiro-2026Q3.json");
  respond({ cmd_import_open: () => reviewView() });
  renderApp("/imports");
  expect(await screen.findByText("No import in review")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Open batch file" }));

  expect(mockInvoke).toHaveBeenCalledWith("cmd_import_open", { path: "/tmp/degiro-2026Q3.json" });
  expect(await screen.findByText("Review · degiro-2026Q3.json")).toBeInTheDocument();
  expect(screen.getByText("degiro-csv 0.4.0")).toBeInTheDocument();
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
  await userEvent.click(await screen.findByRole("button", { name: "Open batch file" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("mizan.import/9");
  expect(screen.getByText("No import in review")).toBeInTheDocument();
});

test("cancelling the picker does nothing", async () => {
  mockOpen.mockResolvedValue(null);
  respond({});
  renderApp("/imports");
  await userEvent.click(await screen.findByRole("button", { name: "Open batch file" }));
  expect(mockInvoke).not.toHaveBeenCalledWith("cmd_import_open", expect.anything());
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
