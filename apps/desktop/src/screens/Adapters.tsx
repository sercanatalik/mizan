import { open, save } from "@tauri-apps/plugin-dialog";
import { useState } from "react";
import { useNavigate } from "react-router";
import { PageHeader } from "../app/Layout";
import type { MapRequest } from "../app/useMenuEvents";
import { errorMessage } from "../bridge/api";
import type { AdapterRow } from "../bridge/generated/AdapterRow";
import { useAdapterMutations, useAdapters } from "../bridge/queries";
import { Badge, Button, Card, DataTable } from "../ui";

const muted = { fontSize: 12, color: "var(--text-muted)" } as const;
const input = { font: "inherit", fontSize: 13, padding: "5px 8px", borderRadius: 6, border: "1px solid var(--border-strong)" } as const;
const KINDS: Record<string, string> = { native: "Native", script: "Script", mapping: "Mapping" };

function Actions({ a }: { a: AdapterRow }) {
  const m = useAdapterMutations();
  const [mode, setMode] = useState<"idle" | "rename" | "delete">("idle");
  const [name, setName] = useState(a.name);
  const saved = a.origin === "saved";
  const exportIt = async () => {
    const dest = await save({ title: `Export ${a.name}`, defaultPath: `${a.id}.toml`, filters: [{ name: "Adapter manifest", extensions: ["toml"] }] });
    if (typeof dest === "string") m.exportTo.mutate({ id: a.id, dest });
  };
  const error = m.rename.error ?? m.remove.error ?? m.exportTo.error;
  if (mode === "rename") {
    return (
      <form
        style={{ display: "inline-flex", gap: 6, alignItems: "center", flexWrap: "wrap", justifyContent: "flex-end" }}
        onSubmit={(e) => {
          e.preventDefault();
          m.rename.mutate({ id: a.id, name }, { onSuccess: () => setMode("idle") });
        }}
      >
        <label className="visually-hidden" htmlFor={`rename-${a.id}`}>
          New name for {a.name}
        </label>
        <input id={`rename-${a.id}`} style={input} value={name} onChange={(e) => setName(e.target.value)} autoFocus />
        <Button small type="submit" variant="primary" disabled={!name.trim() || m.rename.isPending}>
          Save
        </Button>
        <Button small variant="ghost" onClick={() => setMode("idle")}>
          Cancel
        </Button>
        {error && <span role="alert" style={{ ...muted, color: "var(--warn-strong)" }}>{errorMessage(error)}</span>}
      </form>
    );
  }
  if (mode === "delete") {
    return (
      <span role="group" aria-label={`Delete ${a.name}?`} style={{ display: "inline-flex", gap: 6, alignItems: "center", flexWrap: "wrap", justifyContent: "flex-end" }}>
        <span style={{ fontSize: 13 }}>Delete {a.name}? Imports already made with it are kept.</span>
        <Button small variant="primary" onClick={() => m.remove.mutate(a.id)} disabled={m.remove.isPending}>
          Delete
        </Button>
        <Button small variant="ghost" onClick={() => setMode("idle")}>
          Cancel
        </Button>
        {error && <span role="alert" style={{ ...muted, color: "var(--warn-strong)" }}>{errorMessage(error)}</span>}
      </span>
    );
  }
  return (
    <span style={{ display: "inline-flex", gap: 6, flexWrap: "wrap", justifyContent: "flex-end", alignItems: "center" }}>
      {a.id && (
        <Button small onClick={() => void exportIt()} aria-label={`Export ${a.name}`}>
          Export
        </Button>
      )}
      {saved && a.id && (
        <>
          <Button small onClick={() => setMode("rename")} aria-label={`Rename ${a.name}`}>
            Rename
          </Button>
          <Button small variant="ghost" onClick={() => setMode("delete")} aria-label={`Delete ${a.name}`}>
            Delete
          </Button>
        </>
      )}
      {m.exportTo.isSuccess && <span style={muted}>Exported</span>}
      {error && <span role="alert" style={{ ...muted, color: "var(--warn-strong)" }}>{errorMessage(error)}</span>}
    </span>
  );
}

export function Adapters() {
  const { data } = useAdapters();
  const navigate = useNavigate();
  const newMapping = async () => {
    const path = await open({ title: "Choose a CSV or XLSX file to map", filters: [{ name: "Spreadsheets", extensions: ["csv", "txt", "xlsx"] }] });
    if (typeof path !== "string") return;
    const fileName = path.split(/[\\/]/).pop() ?? path;
    const fileKind = fileName.toLowerCase().endsWith(".xlsx") ? "xlsx" : "csv";
    navigate("/imports", { state: { map: { path, fileName, fileKind, at: Date.now() } } satisfies MapRequest });
  };
  const header = (
    <PageHeader
      title="Adapters"
      subtitle="Readers for statement files: built into Mizan, or mappings you saved"
      actions={
        <Button variant="primary" onClick={() => void newMapping()}>
          New mapping from file…
        </Button>
      }
    />
  );
  if (!data) return header;
  return (
    <>
      {header}
      <Card label="Adapters">
        <DataTable label="Adapters" minWidth={880}>
          <thead>
            <tr>
              <th scope="col">Adapter</th>
              <th scope="col">Type</th>
              <th scope="col">Files</th>
              <th scope="col">Origin</th>
              <th scope="col" style={{ textAlign: "right" }}>
                Imports
              </th>
              <th scope="col" style={{ textAlign: "right" }}>
                <span className="visually-hidden">Actions</span>
              </th>
            </tr>
          </thead>
          <tbody>
            {data.adapters.map((a) => (
              <tr key={`${a.origin}-${a.id || a.name}`} data-testid="adapter-row">
                <td>
                  <div style={{ fontWeight: 500 }}>{a.name}</div>
                  {a.id && (
                    <div className="num" style={muted}>
                      {a.id} · {a.version}
                    </div>
                  )}
                  {a.description && <div style={muted}>{a.description}</div>}
                  {a.error && (
                    <div style={{ fontSize: 12, color: "var(--warn-strong)" }} data-testid="adapter-error">
                      Not loaded: {a.error}
                    </div>
                  )}
                </td>
                <td>{KINDS[a.kind] ?? "—"}</td>
                <td style={{ color: "var(--text-secondary)" }}>{a.files.map((f) => f.toUpperCase()).join(", ") || "—"}</td>
                <td>{a.origin === "saved" ? <Badge tone="accent">Saved</Badge> : <Badge>Built in</Badge>}</td>
                <td className="num" style={{ textAlign: "right" }}>
                  {a.imports}
                </td>
                <td style={{ textAlign: "right" }}>
                  <Actions a={a} />
                </td>
              </tr>
            ))}
          </tbody>
        </DataTable>
      </Card>
    </>
  );
}
