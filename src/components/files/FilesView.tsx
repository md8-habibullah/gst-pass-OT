import { PageBody, PageHeader } from "../shell/PageHeader";
import { FileList } from "./FileList";

export function FilesView() {
  return (
    <PageBody wide>
      <PageHeader title="Files" back subtitle="Images, PDFs, key files and documents, kept encrypted next to your passwords." />
      <FileList itemId={null} emptyText="No files yet. Add one and it is encrypted before it touches the disk." />
    </PageBody>
  );
}
