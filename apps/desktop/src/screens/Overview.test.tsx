import { screen, within } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Overview } from "../bridge/generated/Overview";
import { EMPTY, invokeFrom, renderApp } from "../test/render";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

function serve(o: Overview) {
  vi.mocked(invoke).mockImplementation(invokeFrom({ ...EMPTY, cmd_overview: o }) as typeof invoke);
}

beforeEach(() => {
  vi.mocked(invoke).mockReset();
  vi.mocked(listen).mockImplementation((async () => () => {}) as unknown as typeof listen);
});

const overview: Overview = {
  accounts: [
    { account_id: 1, name: "Interactive Brokers", activity_count: 64, latest_check: { as_of: "2026-10-31", passed: true, failed: [], batch_id: 3 } },
    {
      account_id: 2,
      name: "Degiro",
      activity_count: 51,
      latest_check: {
        as_of: "2026-09-30",
        passed: false,
        batch_id: 4,
        failed: [
          { label: "isin:NL0010273215", display: "ASML", expected: "12", computed: "10" },
          { label: "EUR", display: "cash", expected: "2366.20", computed: "3650.80" },
        ],
      },
    },
    { account_id: 3, name: "Savings", activity_count: 1, latest_check: null },
  ],
  attention: [
    { type: "pending_decisions", count: 2 },
    {
      type: "failed_checks",
      account_id: 2,
      account_name: "Degiro",
      as_of: "2026-09-30",
      failed: [{ label: "isin:NL0010273215", display: "ASML", expected: "12", computed: "10" }],
    },
    { type: "unknown_basis", account_id: 2, account_name: "Degiro", instrument_key: "isin:IE00BK5BQT80", display: "VWCE", units: "20" },
  ],
};

test("value area says market data is needed and shows no figures", async () => {
  serve(overview);
  renderApp("/");
  const value = await screen.findByRole("region", { name: "Portfolio value" });
  expect(value).toHaveTextContent("Market data needed");
  expect(value.textContent).not.toMatch(/\d/);
});

test("accounts show passing, failing and no-check status", async () => {
  serve(overview);
  renderApp("/");
  const rows = await screen.findAllByTestId("account-row");
  expect(rows[0]).toHaveTextContent("Statement to 31 Oct 2026 · all balances match");
  expect(rows[1]).toHaveTextContent("2 balance checks failed · ASML, cash");
  expect(within(rows[1]!).getByRole("img", { name: "Warning" })).toBeInTheDocument();
  expect(rows[2]).toHaveTextContent("No balance check yet");
});

test("needs-attention items link to where they are fixed", async () => {
  serve(overview);
  renderApp("/");
  const items = await screen.findAllByTestId("attention-item");
  expect(items[0]).toHaveTextContent("2 records need your decision");
  expect(within(items[0]!).getByRole("link", { name: "Review" })).toHaveAttribute("href", "/imports");
  expect(items[1]).toHaveTextContent("Degiro balance checks failed");
  expect(items[1]).toHaveTextContent("ASML: statement 12, Mizan 10");
  expect(items[2]).toHaveTextContent("Unknown cost basis · 20 VWCE in Degiro");
  expect(within(items[2]!).getByRole("link")).toHaveAttribute("href", "/holdings");
});

test("everything in order", async () => {
  serve({ accounts: [overview.accounts[0]!], attention: [] });
  renderApp("/");
  expect(await screen.findByText("Everything is in order.")).toBeInTheDocument();
});
