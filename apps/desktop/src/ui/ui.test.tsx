import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";
import { Badge, Button, Card, DataTable, EmptyState, Segmented } from ".";

test("Card renders title and content", () => {
  render(<Card title="Accounts">body</Card>);
  expect(screen.getByRole("heading", { name: "Accounts" })).toBeInTheDocument();
  expect(screen.getByText("body")).toBeInTheDocument();
});

test("Badge renders text", () => {
  render(<Badge tone="warn">Your call</Badge>);
  expect(screen.getByText("Your call")).toBeInTheDocument();
});

test("Segmented marks the selected option and reports changes", async () => {
  const onChange = vi.fn();
  render(
    <Segmented
      label="Group by"
      value="acct"
      onChange={onChange}
      options={[
        { value: "acct", label: "Account" },
        { value: "cls", label: "Asset class", count: 3 },
      ]}
    />,
  );
  expect(screen.getByRole("group", { name: "Group by" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Account" })).toHaveAttribute("aria-pressed", "true");
  await userEvent.click(screen.getByRole("button", { name: /Asset class/ }));
  expect(onChange).toHaveBeenCalledWith("cls");
});

test("DataTable renders a labelled table", () => {
  render(
    <DataTable label="Positions">
      <tbody>
        <tr>
          <td>AAPL</td>
        </tr>
      </tbody>
    </DataTable>,
  );
  expect(screen.getByRole("table", { name: "Positions" })).toBeInTheDocument();
});

test("EmptyState renders message and action link", () => {
  render(
    <MemoryRouter>
      <EmptyState title="Nothing yet" message="Import a statement." action={{ to: "/imports", label: "Go to Imports" }} />
    </MemoryRouter>,
  );
  expect(screen.getByRole("status")).toHaveTextContent("Import a statement.");
  expect(screen.getByRole("link", { name: "Go to Imports" })).toHaveAttribute("href", "/imports");
});

test("Button is a real button and can be disabled", () => {
  render(<Button disabled>Commit</Button>);
  expect(screen.getByRole("button", { name: "Commit" })).toBeDisabled();
});
