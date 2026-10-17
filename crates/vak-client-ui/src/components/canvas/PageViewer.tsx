import * as api from "../../api";
import PageView, { readPage } from "../PageView";
import { createLoader } from "./createLoader";
import LoadState from "./LoadState";
import type { ViewerProps } from "./types";

/** A Library page at full size (docs/design/98 §12), at its newest version:
 *  a living page shows its latest run's answer on every reload. */
export default function PageViewer(props: ViewerProps) {
  const artifactId = () => (props.subject.kind === "page" ? props.subject.artifactId : "");
  const loader = createLoader(
    artifactId,
    async (id) => {
      const artifact = await api.libraryArtifact(id);
      if (!artifact.head) throw new Error("This page has no version yet; its first comes from its automation's first run.");
      const page = readPage(await api.libraryVersionBytes(id, artifact.head));
      if (!page) throw new Error("This Library item is not a page.");
      return page;
    },
    undefined,
    { reloadOn: () => props.reloadKey },
  );
  return <LoadState loader={loader} label="Opening the page…">{(page) => <PageView page={page} />}</LoadState>;
}
