/** Dropping files anywhere on the conversation hands them to the composer,
 *  which saves them to the workspace inbox (docs/design/72, "File in"). */
export const ATTACH_FILES_EVENT = "vak:attach-files";

export function attachFiles(files: File[]): void {
  window.dispatchEvent(new CustomEvent(ATTACH_FILES_EVENT, { detail: { files } }));
}

/** The name a file had when it was attached: the inbox prefixes each saved
 *  name with twelve hex digits of its digest. Other paths keep their name. */
export function displayFileName(path: string): string {
  const name = path.split("/").pop() ?? path;
  return path.startsWith("inbox/") ? name.replace(/^[0-9a-f]{12}-/, "") : name;
}

/** Attaching a Library artifact (plan M8.3b): the composer shows it as a
 *  chip and the request names it; the server writes what the model is told. */
export const ATTACH_ARTIFACT_EVENT = "vak:attach-artifact";

export interface ArtifactChip {
  id: string;
  name: string;
  mode: "continue" | "another";
}

export function attachArtifact(chip: ArtifactChip): void {
  window.dispatchEvent(new CustomEvent(ATTACH_ARTIFACT_EVENT, { detail: chip }));
}
