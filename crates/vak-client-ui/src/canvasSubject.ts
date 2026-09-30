// What the Canvas is showing, named by identity rather than by path
// (docs/design/66, docs/plans/canvas-plan.md K1).
//
// A path alone is ambiguous: the same relative name can be a workspace file,
// a file one run left in scratch, or a version of a saved draft, and each is
// read through a different route. A subject carries the identity that picks
// the route, so opening one can never quietly show a different file. There is
// no fallback between kinds: a file that cannot be read is an error, never
// replaced by text found somewhere else.

import { isOfficePath } from "./officeFiles.ts";

export type ArtifactDisplayType = "html" | "pdf" | "image" | "table" | "code" | "server" | "office";

interface SubjectBase {
  title: string;
  /** The answer in the conversation this belongs to, so the Canvas can return to it. */
  resultId?: string;
  /** A cited place in an Office file or PDF to open at (`path#anchor`). */
  anchor?: string;
}

export type CanvasSubject =
  /** A file in the workspace, read through the workspace file route. */
  | (SubjectBase & { kind: "file"; path: string; sessionId?: string })
  /** A file one run left in its scratch directory. */
  | (SubjectBase & { kind: "execution_artifact"; path: string; sessionId: string; executionId: string })
  /** A file of one saved version of a draft, read by hash-checked candidate routes. */
  | (SubjectBase & { kind: "draft_file"; path: string; sessionId: string; candidateId: string; executionId?: string })
  /** Markup shown from the conversation itself, with no file behind it. */
  | (SubjectBase & { kind: "inline"; html: string; basePath?: string })
  /** A dev server the session's launch configuration names. */
  | (SubjectBase & { kind: "live_server"; serverName: string; sessionId: string; candidateId?: string; path?: string });

/** The identity a file was opened from; the shape decides which route reads it. */
export type ArtifactOrigin =
  | { candidateId: string; sessionId: string; executionId?: string; resultId?: string; anchor?: string }
  | { candidateId?: undefined; executionId: string; sessionId: string; resultId?: string; anchor?: string }
  | { candidateId?: undefined; executionId?: undefined; sessionId?: string; resultId?: string; anchor?: string };

/** The origin of a file named by the run that made it, when the conversation is known. */
export function runOrigin(from: { sessionId?: string; executionId?: string; resultId?: string; anchor?: string }): ArtifactOrigin {
  const { sessionId, executionId, resultId, anchor } = from;
  return sessionId && executionId ? { sessionId, executionId, resultId, anchor } : { sessionId, resultId, anchor };
}

export function fileSubject(path: string, origin: ArtifactOrigin, title: string): CanvasSubject {
  const { resultId, anchor } = origin;
  if (origin.candidateId) {
    return { kind: "draft_file", title, path, sessionId: origin.sessionId, candidateId: origin.candidateId, executionId: origin.executionId, resultId, anchor };
  }
  if (origin.executionId) {
    return { kind: "execution_artifact", title, path, sessionId: origin.sessionId, executionId: origin.executionId, resultId, anchor };
  }
  return { kind: "file", title, path, sessionId: origin.sessionId, resultId, anchor };
}

/** The saved file or the workspace path a subject reads; empty when there is none. */
export function subjectPath(subject: CanvasSubject): string {
  switch (subject.kind) {
    case "inline": return subject.basePath ?? "";
    case "live_server": return subject.path ?? "";
    default: return subject.path;
  }
}

export function subjectSessionId(subject: CanvasSubject): string | undefined {
  return subject.kind === "inline" ? undefined : subject.sessionId;
}

export function subjectCandidateId(subject: CanvasSubject): string | undefined {
  return subject.kind === "draft_file" || subject.kind === "live_server" ? subject.candidateId : undefined;
}

export function subjectExecutionId(subject: CanvasSubject): string | undefined {
  return subject.kind === "draft_file" || subject.kind === "execution_artifact" ? subject.executionId : undefined;
}

const IMAGE_EXTENSIONS = /\.(png|jpe?g|gif|webp|svg|ico|bmp)$/;

/** How a subject is drawn. Files are told apart by extension; nothing else is guessed. */
export function displayType(subject: CanvasSubject): ArtifactDisplayType {
  if (subject.kind === "live_server") return "server";
  if (subject.kind === "inline") return "html";
  const path = subject.path.toLowerCase();
  if (path.endsWith(".pdf")) return "pdf";
  if (path.endsWith(".csv") || path.endsWith(".tsv")) return "table";
  if (isOfficePath(path)) return "office";
  if (IMAGE_EXTENSIONS.test(path)) return "image";
  if (/\.(html?|xhtml)$/.test(path)) return "html";
  return "code";
}

/** Whether the subject is a document that reads best with the whole viewport. */
export function isDocumentSubject(subject: CanvasSubject): boolean {
  return subject.kind !== "inline" && subject.kind !== "live_server" && /\.(docx|xlsx|pptx|pdf)(?:$|[?#])/i.test(subject.path);
}

const bare = (path: string) => path.trim().replace(/^\.\//, "");

export type ExecutionMatch =
  | { kind: "none" }
  | { kind: "one"; executionId: string }
  | { kind: "many"; count: number };

/**
 * Which runs left exactly this path. Only an identical path counts: a shared
 * file name or a shared suffix says nothing about which run a reader means,
 * and guessing shows a different turn's file.
 */
export function matchExecutionArtifact(path: string, executions: ReadonlyArray<{ id: string; artifacts: ReadonlyArray<{ path: string }> }>): ExecutionMatch {
  const wanted = bare(path);
  const ids = executions.filter((run) => run.artifacts.some((artifact) => bare(artifact.path) === wanted)).map((run) => run.id);
  if (ids.length === 0) return { kind: "none" };
  return ids.length === 1 ? { kind: "one", executionId: ids[0] } : { kind: "many", count: ids.length };
}
