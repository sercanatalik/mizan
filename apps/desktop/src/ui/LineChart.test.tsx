import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { LineChart } from ".";

const dates = ["2026-01-01", "2026-01-02", "2026-01-03", "2026-02-01"];

function chart(extra: Partial<Parameters<typeof LineChart>[0]> = {}) {
  return render(
    <LineChart
      label="Growth"
      dates={dates}
      series={[
        { name: "Portfolio", values: [100, 101, null, 104], labels: ["100.00", "101.00", null, "104.00"] },
        { name: "AFT", values: [100, 99, 98, 102] },
      ]}
      estimated={[true, true, false, false]}
      format={(v) => v.toFixed(0)}
      {...extra}
    />,
  );
}

test("renders both series with a legend and an estimated band", () => {
  chart();
  expect(screen.getByTestId("series-0").getAttribute("d")).toMatch(/^M.*L.*M/); // gap splits the line
  expect(screen.getByTestId("series-1")).toBeInTheDocument();
  const legend = screen.getByRole("list", { name: "Growth legend" });
  expect(legend).toHaveTextContent("Portfolio");
  expect(legend).toHaveTextContent("AFT");
  expect(legend).toHaveTextContent("Estimated values");
  expect(screen.getAllByTestId("estimated-band")).toHaveLength(1);
});

test("has an accessible summary and a table view", async () => {
  chart();
  const img = screen.getByRole("img");
  expect(img).toHaveAccessibleName(/Growth, 2026-01-01 to 2026-02-01\. Portfolio: 104\.00 at the end; AFT: 102 at the end\./);
  await userEvent.click(screen.getByText("Show as table"));
  const table = screen.getByRole("table", { name: "Growth table" });
  // One row per month: 3 Jan (last of January) and 1 Feb.
  expect(table.querySelectorAll("tbody tr")).toHaveLength(2);
  expect(table).toHaveTextContent("104.00");
});

test("keyboard hover shows a readout with both values", async () => {
  chart();
  const img = screen.getByRole("img");
  img.focus();
  await userEvent.keyboard("{Home}");
  const tip = screen.getByRole("status");
  expect(tip).toHaveTextContent("2026-01-01 · estimated");
  expect(tip).toHaveTextContent("Portfolio100.00");
  await userEvent.keyboard("{ArrowRight}{ArrowRight}");
  expect(screen.getByRole("status")).toHaveTextContent("Portfolio—");
  fireEvent.keyDown(img, { key: "Escape" });
  expect(screen.queryByRole("status")).not.toBeInTheDocument();
});

test("a single series has no legend", () => {
  render(<LineChart label="Value" dates={dates} series={[{ name: "Value", values: [1, 2, 3, 4] }]} format={String} />);
  expect(screen.queryByRole("list")).not.toBeInTheDocument();
});
