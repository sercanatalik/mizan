import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Contributions } from "../bridge/generated/Contributions";
import { EMPTY, invokeFrom, renderApp } from "../test/render";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

const acct = (from: string, amount: string) => ({ account_id: 1, name: "Interactive Brokers", from, by_currency: [{ currency: "USD", amount }] });
const contributions: Contributions = {
  periods: [
    { period: "ytd", to: "2026-11-01", accounts: [acct("2026-01-01", "+8500.00")], total: [{ currency: "USD", amount: "+8500.00" }] },
    { period: "last12_months", to: "2026-11-01", accounts: [acct("2025-11-02", "+8500.00")], total: [{ currency: "USD", amount: "+8500.00" }] },
    { period: "since_start", to: "2026-11-01", accounts: [acct("2025-03-01", "+10500.00")], total: [{ currency: "USD", amount: "+10500.00" }] },
  ],
};

beforeEach(() => {
  vi.mocked(invoke).mockReset();
  vi.mocked(invoke).mockImplementation(invokeFrom({ ...EMPTY, cmd_contributions: contributions }) as typeof invoke);
  vi.mocked(listen).mockImplementation((async () => () => {}) as unknown as typeof listen);
});

test("returns show the engine empty state and no percentages", async () => {
  renderApp("/performance");
  const returns = await screen.findByRole("region", { name: "Returns" });
  expect(within(returns).getAllByText("Needs the returns engine and market data.")).toHaveLength(3);
  expect(screen.getByRole("region", { name: "Growth chart" })).toHaveTextContent("Returns engine needed");
  expect(document.body.textContent).not.toMatch(/%/);
});

test("net contributions per account and total, switching periods", async () => {
  renderApp("/performance");
  const row = await screen.findByTestId("contribution-row");
  expect(row).toHaveTextContent("+8500.00 USD");
  expect(row).toHaveTextContent("2026-01-01");
  expect(screen.getByTestId("contribution-total")).toHaveTextContent("+8500.00 USD");

  await userEvent.click(screen.getByRole("button", { name: "Since start" }));
  expect(screen.getByTestId("contribution-row")).toHaveTextContent("+10500.00 USD");
  expect(screen.getByTestId("contribution-row")).toHaveTextContent("2025-03-01");
  expect(screen.getByRole("button", { name: "Since start" })).toHaveAttribute("aria-pressed", "true");

  await userEvent.click(screen.getByRole("button", { name: "12M" }));
  expect(screen.getByTestId("contribution-row")).toHaveTextContent("2025-11-02");
});
