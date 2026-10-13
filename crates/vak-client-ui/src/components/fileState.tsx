import type { VersionNow } from "../api";

/** A version's state in everyday words, and the mark it carries (doc 90 §5).
 *  `from` is where the version came from: an edit found in the folder is the
 *  person's, outside Vak. The Library page and a conversation's card both
 *  say it this way. */
export function stateOf(now: VersionNow | null | undefined, from?: string): { word: string; mark: string } | null {
  if (!now) return null;
  switch (now.is) {
    case "in_folder": return from === "workspace"
      ? { word: "Changed outside Vak · in your folder", mark: "changed" }
      : { word: "In your folder", mark: "current" };
    case "accepted": return { word: "Accepted", mark: "accepted" };
    case "written": return { word: "Written to your folder", mark: "accepted" };
    case "found_outside": return { word: "Changed outside Vak", mark: "changed" };
    case "waiting": return { word: "Waiting for you", mark: "waiting" };
    case "revised": return { word: "Revised", mark: "accepted" };
    case "undone": return { word: "Undone by you", mark: "undone" };
    case "saved": return { word: "Kept", mark: "accepted" };
    case "in_trash": return { word: "In the trash", mark: "trash" };
    case "erased": return { word: "Deleted for good", mark: "trash" };
  }
}

export function Pill(props: { mark: string; children: string }) {
  return <span class={`file-pill st-${props.mark}`}><span class="dot" aria-hidden="true" />{props.children}</span>;
}
