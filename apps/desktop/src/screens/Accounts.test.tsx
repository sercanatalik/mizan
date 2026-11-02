import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { AccountsList } from "../bridge/generated/AccountsList";
import { EMPTY, invokeFrom, renderApp } from "../test/render";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
const mockInvoke = vi.mocked(invoke);

beforeEach(() => {
  vi.mocked(listen).mockResolvedValue(() => {});
});

const list: AccountsList = {
  base_currency: "TRY",
  accounts: [
    {
      id: 1,
      name: "BES pension",
      value: "-98502.89",
      activities: 412,
      identifiers: ["BES-A"],
      tax_treatment: "pension",
      latest_check: { as_of: "2026-10-31", passed: false, failed: ["AEA"] },
      merge: null,
    },
    { id: 2, name: "IBKR", value: "250000.00", activities: 80, identifiers: ["U1234567"], tax_treatment: "taxable", latest_check: null, merge: null },
  ],
  merged: [
    {
      id: 3,
      name: "IBKR old",
      value: null,
      activities: 0,
      identifiers: [],
      tax_treatment: "taxable",
      latest_check: null,
      merge: { merge_id: 7, into_id: 2, into_name: "IBKR", merged_at: "2026-11-01T10:00:00Z" },
    },
  ],
};

function serve(extra: Record<string, unknown> = {}) {
  mockInvoke.mockImplementation(invokeFrom({ ...EMPTY, cmd_accounts_list: list, cmd_account_undo_merge: [], ...extra }) as typeof invoke);
}

test("Accounts lists accounts with links and merged accounts with undo", async () => {
  serve();
  renderApp("/accounts");
  const rows = await screen.findAllByTestId("account-list-row");
  expect(rows).toHaveLength(2);
  expect(within(rows[0]!).getByRole("link", { name: "BES pension" })).toHaveAttribute("href", "/accounts/1");
  expect(rows[0]).toHaveTextContent("−98,502.89");
  expect(rows[0]).toHaveTextContent("Mismatch on 31 Oct 2026: AEA");
  expect(rows[1]).toHaveTextContent("No balance checks");
  const merged = screen.getByTestId("merged-row");
  expect(merged).toHaveTextContent("IBKR old");
  await userEvent.click(within(merged).getByRole("button", { name: "Undo merge" }));
  await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("cmd_account_undo_merge", { merge_id: 7 }));
});

test("undo reports identifiers that stayed where they were moved", async () => {
  serve({ cmd_account_undo_merge: ["U7654321"] });
  renderApp("/accounts");
  await userEvent.click(await screen.findByRole("button", { name: "Undo merge" }));
  expect(await screen.findByRole("note")).toHaveTextContent("U7654321");
});

test("no accounts points to Imports", async () => {
  serve({ cmd_accounts_list: { base_currency: "TRY", accounts: [], merged: [] } });
  renderApp("/accounts");
  expect(await screen.findByText("No accounts yet")).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Go to Imports" })).toHaveAttribute("href", "/imports");
});
