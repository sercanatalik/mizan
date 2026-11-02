import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { InboxItem } from "../bridge/generated/InboxItem";
import type { InboxView } from "../bridge/generated/InboxView";
import { EMPTY, invokeFrom, renderApp } from "../test/render";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
const mockInvoke = vi.mocked(invoke);
vi.mocked(listen);

const item = (i: Partial<InboxItem> & Pick<InboxItem, "key" | "kind" | "title" | "action">): InboxItem => ({
  subtitle: "",
  body: "",
  facts: [],
  check_lines: [],
  since: null,
  fingerprint: "f",
  snoozed_until: null,
  ...i,
});

const items: InboxItem[] = [
  item({ key: "import:decisions", kind: "import", title: "3 records awaiting a decision", subtitle: "flex_2026-10.xml", since: "2026-11-02", action: { type: "import_review" } }),
  item({
    key: "check:4",
    kind: "check",
    title: "Balance check failed · TR Bank",
    subtitle: "cash on 2026-09-30",
    since: "2026-09-30",
    action: { type: "account_checks", account_id: 4 },
    check_lines: [{ label: "TRY", display: "cash", expected: "42310.00", computed: "41060.00", difference: "1250.00" }],
  }),
  item({
    key: "check:5",
    kind: "check",
    title: "Balance check failed · IBKR",
    since: "2026-09-28",
    action: { type: "account_checks", account_id: 5 },
  }),
  item({ key: "price:sym:AFT@", kind: "price", title: "No price for 12 days · AFT", since: "2026-10-21", action: { type: "asset_price", asset_id: 9 } }),
  item({ key: "cost:2:isin:IE00BK5BQT80", kind: "cost", title: "Unknown cost basis · 20 VWCE in Degiro", action: { type: "add_position_in", account_id: 2, asset_id: 3 } }),
];

function serve(view: InboxView, extra: Record<string, unknown> = {}) {
  mockInvoke.mockImplementation(invokeFrom({ ...EMPTY, cmd_inbox: view, cmd_inbox_count: view.items.length, ...extra }) as typeof invoke);
}

beforeEach(() => {
  mockInvoke.mockReset();
  serve({ items, snoozed: [] });
});

test("filter by type with counts", async () => {
  renderApp("/inbox");
  const filter = await screen.findByRole("group", { name: "Filter" });
  expect(within(filter).getByRole("button", { name: /^All ?5$/ })).toHaveAttribute("aria-pressed", "true");
  await userEvent.click(within(filter).getByRole("button", { name: /^Balance checks ?2$/ }));
  const list = screen.getByRole("region", { name: "Items" });
  const titles = within(list).getAllByRole("button").map((b) => b.textContent);
  expect(titles).toHaveLength(2);
  expect(titles.every((t) => t?.startsWith("Balance check failed"))).toBe(true);
});

test("the first item is selected and its detail shows; a failed check lists its lines", async () => {
  renderApp("/inbox");
  const detail = await screen.findByRole("region", { name: "Item detail" });
  expect(within(detail).getByRole("heading", { name: "3 records awaiting a decision" })).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: /TR Bank/ }));
  const d2 = screen.getByRole("region", { name: "Item detail" });
  const lines = within(d2).getByRole("table", { name: "Failing lines" });
  const row = within(lines).getByRole("row", { name: /TRY cash/ });
  expect(row).toHaveTextContent("42,310.00");
  expect(row).toHaveTextContent("41,060.00");
  expect(row).toHaveTextContent("1,250.00");
});

test.each([
  ["3 records awaiting a decision", "Open the review", "/imports", null],
  ["TR Bank", "Open balance checks", "/accounts/4", "#balance-checks"],
  ["AFT", "Set the price", "/assets", null],
  ["VWCE", "Record the cost", "/activity", null],
])("primary action of %s", async (name, action, path, hash) => {
  serve({ items, snoozed: [] }, { cmd_activity_options: { ...EMPTY.cmd_activity_options, accounts: [{ id: 2, name: "Degiro" }], assets: [{ id: 3, label: "VWCE", currency: "EUR" }] }, cmd_activity_check: { errors: [], warnings: [], duplicates: [] } });
  const { router } = renderApp("/inbox");
  await userEvent.click(await screen.findByRole("button", { name: new RegExp(name) }));
  await userEvent.click(within(screen.getByRole("region", { name: "Item detail" })).getByRole("button", { name: action }));
  await waitFor(() => expect(router.state.location.pathname).toBe(path));
  if (hash) expect(router.state.location.hash).toBe(hash);
  if (path === "/assets") expect(router.state.location.state).toEqual({ editAsset: 9 });
  if (path === "/activity") {
    const form = await screen.findByRole("region", { name: "Activity form" });
    expect(within(form).getByRole("combobox", { name: "Account of activity" })).toHaveValue("2");
  }
});

test("snooze an item for a week", async () => {
  const snoozed = { ...items[4]!, snoozed_until: "2026-11-09T10:00:00Z" };
  let view: InboxView = { items, snoozed: [] };
  mockInvoke.mockImplementation(
    invokeFrom({
      ...EMPTY,
      cmd_inbox: () => view,
      cmd_inbox_count: () => view.items.length,
      cmd_inbox_snooze: () => {
        view = { items: items.slice(0, 4), snoozed: [snoozed] };
        return view;
      },
      cmd_inbox_unsnooze: undefined,
    }) as typeof invoke,
  );
  renderApp("/inbox");
  await userEvent.click(await screen.findByRole("button", { name: /VWCE/ }));
  await userEvent.click(screen.getByRole("button", { name: "Snooze 7 days" }));
  expect(mockInvoke).toHaveBeenCalledWith("cmd_inbox_snooze", { key: "cost:2:isin:IE00BK5BQT80", days: 7 });
  const s = await screen.findByRole("region", { name: "Snoozed" });
  expect(within(s).getByText("until 9 Nov 2026")).toBeInTheDocument();
  expect(within(screen.getByRole("region", { name: "Items" })).queryByText(/VWCE/)).toBeNull();
  await waitFor(() => expect(screen.getByLabelText("4 to review")).toBeInTheDocument());
  await userEvent.click(within(s).getByRole("button", { name: "Unsnooze" }));
  expect(mockInvoke).toHaveBeenCalledWith("cmd_inbox_unsnooze", { key: "cost:2:isin:IE00BK5BQT80" });
});

test("empty Inbox says every figure is backed by checked records", async () => {
  serve({ items: [], snoozed: [] });
  renderApp("/inbox");
  const status = await screen.findByRole("status");
  expect(status).toHaveTextContent("Everything is in order");
  expect(status).toHaveTextContent("Every figure is backed by checked records");
});

test("an item can be selected through the location (Overview preview)", async () => {
  const { router } = renderApp("/");
  await router.navigate("/inbox", { state: { select: "price:sym:AFT@" } });
  const detail = await screen.findByRole("region", { name: "Item detail" });
  expect(within(detail).getByRole("heading", { name: "No price for 12 days · AFT" })).toBeInTheDocument();
});
