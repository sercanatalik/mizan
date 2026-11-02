import type { MapperInspect } from "../../bridge/generated/MapperInspect";
import { initialDraft, slug, toMappingDoc } from "./mapping";

const inspect: MapperInspect = {
  file_name: "export.csv",
  file_kind: "csv",
  options: { delimiter: ",", encoding: null, skip_rows: 0, sheet: null },
  sheets: [],
  header: ["Date", "Product", "ISIN", "Quantity", "Amount", "Currency", "Order Id"],
  rows: [],
  total_rows: 0,
};

test("slug folds Turkish letters and punctuation", () => {
  expect(slug("Ziraat Hesap Dökümü (TL)")).toBe("ziraat-hesap-dokumu-tl");
  expect(slug("  ")).toBe("");
});

test("a draft becomes a mapping document without empty fields", () => {
  const d = initialDraft(inspect);
  expect(d.dateFormat).toBe("%Y-%m-%d");
  const doc = toMappingDoc({ ...d, name: "Broker export" }, inspect) as Record<string, any>; // eslint-disable-line @typescript-eslint/no-explicit-any
  expect(doc.adapter.id).toBe("broker-export");
  expect(doc.input).toEqual({ delimiter: ",", skip_rows: 0, decimal: ".", thousands: ",", date_format: "%Y-%m-%d" });
  expect(doc.account).toEqual({ hint: { const: "export" } });
  expect(doc.fields).toEqual({
    date: { column: "Date" },
    quantity: { column: "Quantity", transform: ["abs"] },
    net_cash: { amount: { column: "Amount", transform: ["abs"] }, currency: { column: "Currency" } },
    instrument: { isin: { column: "ISIN" }, name: { column: "Product" } },
    external_id: { column: "Order Id", optional: true },
  });
  expect(doc.detect.required_columns).toEqual(["Date", "Product", "ISIN", "Quantity", "Amount", "Currency", "Order Id"]);
});
