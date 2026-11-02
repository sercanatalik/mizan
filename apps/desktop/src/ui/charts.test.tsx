import { fireEvent, render, screen, within } from "@testing-library/react";
import { CATEGORICAL, Donut, StackedAreaChart } from ".";

const series = [
  { name: "Bono", values: [100, 110, 120] },
  { name: "Altın", values: [null, 50, 55] },
];

test("stacked area chart draws one band per series with a legend and table", () => {
  render(<StackedAreaChart label="Value at current units" dates={["2026-01-01", "2026-01-02", "2026-01-03"]} series={series} format={(v) => v.toFixed(0)} />);
  expect(screen.getByTestId("band-0")).toHaveStyle({ fill: CATEGORICAL[0] });
  expect(screen.getByTestId("band-1")).toHaveStyle({ fill: CATEGORICAL[1] });
  const legend = screen.getByRole("list", { name: "Value at current units legend" });
  expect(within(legend).getAllByRole("listitem").map((li) => li.textContent)).toEqual(["Bono", "Altın"]);
  const table = screen.getByRole("table", { name: "Value at current units table" });
  const rows = within(table).getAllByRole("row");
  expect(rows[1]).toHaveTextContent("2026-01-03");
  expect(rows[1]).toHaveTextContent("175");
  expect(rows[3]).toHaveTextContent("—");
  expect(screen.getByRole("img")).toHaveAccessibleName(/total 175 at the end/);
});

test("stacked area chart tooltip lists each band and the total", () => {
  render(<StackedAreaChart label="Chart" dates={["2026-01-01", "2026-01-02", "2026-01-03"]} series={series} format={(v) => v.toFixed(0)} />);
  const svg = screen.getByRole("img");
  fireEvent.keyDown(svg, { key: "ArrowRight" });
  fireEvent.keyDown(svg, { key: "ArrowRight" });
  const tip = screen.getByRole("status");
  expect(tip).toHaveTextContent("2026-01-02");
  expect(tip).toHaveTextContent("Bono110");
  expect(tip).toHaveTextContent("Altın50");
  expect(tip).toHaveTextContent("Total160");
});

test("donut shows one slice per share with a labelled legend", () => {
  render(
    <Donut
      label="Where the money sits"
      slices={[
        { name: "Bono", value: 37.9, label: "37.9%" },
        { name: "Altın", value: 36.7, label: "36.7%" },
        { name: "Para Piyasası", value: 25.4, label: "25.4%" },
      ]}
    />,
  );
  expect(screen.getByTestId("slice-2")).toHaveStyle({ fill: CATEGORICAL[2] });
  const legend = screen.getByRole("list", { name: "Where the money sits legend" });
  expect(legend).toHaveTextContent("Bono37.9%");
  fireEvent.pointerEnter(screen.getByTestId("slice-1"));
  expect(screen.getByTestId("slice-0")).toHaveStyle({ opacity: "0.45" });
  expect(screen.getByRole("img")).toHaveAccessibleName(/Altın 36.7%/);
});
