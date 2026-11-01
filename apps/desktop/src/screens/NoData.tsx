import { EmptyState } from "../ui";

/** Shown when the database has nothing to compute from yet. */
export function NoData({ what }: { what: string }) {
  return (
    <EmptyState
      title="No data yet"
      message={`Import a statement batch to see ${what}.`}
      action={{ to: "/imports", label: "Go to Imports" }}
    />
  );
}
