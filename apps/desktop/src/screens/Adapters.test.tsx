import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import type { AdapterRow } from "../bridge/generated/AdapterRow";
import type { AdaptersList } from "../bridge/generated/AdaptersList";
import { EMPTY, invokeFrom, renderApp } from "../test/render";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
const mockInvoke = vi.mocked(invoke);
vi.mocked(listen);

const row = (r: Partial<AdapterRow> & Pick<AdapterRow, "id" | "name">): AdapterRow => ({
  version: "1.0.0",
  kind: "mapping",
  files: ["csv"],
  origin: "bundled",
  description: null,
  imports: 0,
  error: null,
  file: null,
  ...r,
});

const list: AdaptersList = {
  can_save: true,
  adapters: [
    row({ id: "ibkr-flex", name: "Interactive Brokers Flex Query", kind: "native", files: ["xml"], imports: 4 }),
    row({ id: "my-bank-csv", name: "My Bank CSV", origin: "saved", imports: 2, file: "my-bank-csv.toml" }),
    row({ id: "", name: "clash.toml", origin: "saved", kind: "", files: [], error: "the id ofx is used by a built-in adapter", file: "clash.toml" }),
  ],
};

beforeEach(() => {
  mockInvoke.mockReset();
  mockInvoke.mockImplementation(
    invokeFrom({
      ...EMPTY,
      cmd_adapters_list: list,
      cmd_mapping_rename: (a: unknown) => ({
        ...list,
        adapters: list.adapters.map((x) => (x.id === "my-bank-csv" ? { ...x, name: (a as { name: string }).name } : x)),
      }),
      cmd_mapping_delete: { ...list, adapters: list.adapters.filter((x) => x.id !== "my-bank-csv") },
      cmd_adapter_export: undefined,
    }) as typeof invoke,
  );
});

const rowOf = async (name: string) => (await screen.findAllByTestId("adapter-row")).find((r) => r.textContent?.includes(name))!;

test("bundled and saved adapters with type, files, origin and imports", async () => {
  renderApp("/adapters");
  const ibkr = await rowOf("Interactive Brokers Flex Query");
  expect(ibkr).toHaveTextContent("Native");
  expect(ibkr).toHaveTextContent("XML");
  expect(ibkr).toHaveTextContent("Built in");
  const mine = await rowOf("My Bank CSV");
  expect(mine).toHaveTextContent("Saved");
  expect(mine).toHaveTextContent("Mapping");
  const bad = await rowOf("clash.toml");
  expect(within(bad).getByTestId("adapter-error")).toHaveTextContent("Not loaded: the id ofx is used by a built-in adapter");
});

test("bundled adapters can be exported but not renamed or deleted", async () => {
  vi.mocked(save).mockResolvedValue("/Users/a/ibkr-flex.toml");
  renderApp("/adapters");
  const ibkr = await rowOf("Interactive Brokers Flex Query");
  expect(within(ibkr).queryByRole("button", { name: /Rename/ })).toBeNull();
  expect(within(ibkr).queryByRole("button", { name: /Delete/ })).toBeNull();
  await userEvent.click(within(ibkr).getByRole("button", { name: "Export Interactive Brokers Flex Query" }));
  await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("cmd_adapter_export", { id: "ibkr-flex", dest: "/Users/a/ibkr-flex.toml" }));
});

test("rename a saved mapping", async () => {
  renderApp("/adapters");
  const mine = await rowOf("My Bank CSV");
  await userEvent.click(within(mine).getByRole("button", { name: "Rename My Bank CSV" }));
  const field = within(mine).getByLabelText("New name for My Bank CSV");
  await userEvent.clear(field);
  await userEvent.type(field, "Garanti CSV");
  await userEvent.click(within(mine).getByRole("button", { name: "Save" }));
  expect(mockInvoke).toHaveBeenCalledWith("cmd_mapping_rename", { id: "my-bank-csv", name: "Garanti CSV" });
  expect(await rowOf("Garanti CSV")).toBeInTheDocument();
});

test("delete asks for confirmation first", async () => {
  renderApp("/adapters");
  const mine = await rowOf("My Bank CSV");
  await userEvent.click(within(mine).getByRole("button", { name: "Delete My Bank CSV" }));
  expect(mockInvoke).not.toHaveBeenCalledWith("cmd_mapping_delete", expect.anything());
  const confirm = within(mine).getByRole("group", { name: "Delete My Bank CSV?" });
  expect(confirm).toHaveTextContent("Imports already made with it are kept");
  await userEvent.click(within(confirm).getByRole("button", { name: "Delete" }));
  expect(mockInvoke).toHaveBeenCalledWith("cmd_mapping_delete", { id: "my-bank-csv" });
  await waitFor(() => expect(screen.queryByText("My Bank CSV")).toBeNull());
});

test("New mapping from file opens the column mapper for it", async () => {
  vi.mocked(open).mockResolvedValue("/Users/a/bank.csv");
  mockInvoke.mockImplementation(
    invokeFrom({ ...EMPTY, cmd_adapters_list: list, cmd_mapper_inspect: () => new Promise(() => {}) }) as typeof invoke,
  );
  const { router } = renderApp("/adapters");
  await userEvent.click(await screen.findByRole("button", { name: "New mapping from file…" }));
  await waitFor(() => expect(router.state.location.pathname).toBe("/imports"));
  expect(await screen.findByText("Column mapper · bank.csv")).toBeInTheDocument();
  await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("cmd_mapper_inspect", expect.objectContaining({ path: "/Users/a/bank.csv" })));
});
