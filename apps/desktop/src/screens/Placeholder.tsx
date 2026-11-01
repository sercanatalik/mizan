import { PageHeader } from "../app/Layout";
import { EmptyState } from "../ui";

export function Placeholder({ title }: { title: string }) {
  return (
    <>
      <PageHeader title={title} />
      <EmptyState title="Not available yet" message={`${title} is planned for a later version of Mizan.`} />
    </>
  );
}
