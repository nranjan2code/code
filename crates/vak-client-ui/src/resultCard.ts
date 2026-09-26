import type { ArtifactStatus, OutputItem } from "./types";

/** The words a result card says about where its file stands
 * (docs/design/75 §6.1). `status` comes from the server's durable records;
 * nothing here guesses it from the file or the answer text. */
export function statusWords(status: ArtifactStatus | null | undefined): { headline: string; folder: string; waiting: boolean } | null {
  switch (status?.state) {
    case "draft": return { headline: `Draft, version ${status.version}, waiting for your review`, folder: "your folder hasn't changed yet", waiting: true };
    case "accepted": return { headline: `Accepted, version ${status.version}`, folder: "now in your folder", waiting: false };
    case "in_folder": return { headline: "Saved in your folder", folder: "", waiting: false };
    default: return null;
  }
}

const KINDS: Array<[RegExp, string]> = [
  [/\.(html?|xhtml)$/i, "Web page"],
  [/\.(png|jpe?g|gif|webp|svg|avif)$/i, "Image"],
  [/\.pdf$/i, "PDF"],
  [/\.(docx|docm|dotx|dotm|doc)$/i, "Word document"],
  [/\.(xlsx|xlsm|xltx|xltm|xls)$/i, "Spreadsheet"],
  [/\.(pptx|pptm|potx|potm|ppt)$/i, "Presentation"],
  [/\.(vsdx|vsdm|vstx|vstm)$/i, "Diagram"],
  [/\.(csv|tsv|json|ya?ml|toml|xml)$/i, "Data"],
  [/\.(md|markdown|txt|rtf)$/i, "Text"],
  [/\.(js|mjs|ts|tsx|jsx|py|rs|go|rb|java|c|cc|cpp|h|css|sh)$/i, "Code"],
];

/** What kind of file this is, in the words a person uses. */
export function fileKind(name: string, mediaType?: string | null): string {
  const found = KINDS.find(([pattern]) => pattern.test(name));
  if (found) return found[1];
  if (mediaType?.startsWith("image/")) return "Image";
  if (mediaType === "text/html") return "Web page";
  if (mediaType?.startsWith("text/")) return "Text";
  return "File";
}

/** The newest file result still waiting for review in a conversation: the
 * one whose Review changes is the primary action. Items are compared by
 * timestamp, then by position, because the server appends saved-draft
 * results after the turns they belong to. */
export function newestWaitingDraft(items: readonly OutputItem[]): string | null {
  let newest: { id: string; at: number; index: number } | null = null;
  items.forEach((item, index) => {
    if (item.content.type !== "artifact" || item.content.artifact.status?.state !== "draft") return;
    if (!item.actions.some((action) => action.verb === "review_draft")) return;
    const at = Date.parse(item.timestamp);
    const time = Number.isNaN(at) ? 0 : at;
    if (!newest || time > newest.at || (time === newest.at && index > newest.index)) newest = { id: item.id, at: time, index };
  });
  return (newest as { id: string } | null)?.id ?? null;
}
