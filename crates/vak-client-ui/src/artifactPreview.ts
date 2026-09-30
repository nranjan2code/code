import * as api from "./api";
import { sandboxedSrcdoc } from "./safeUrl";
import type { CanvasSubject } from "./canvasSubject";

export interface ArtifactPreviewReader {
  readFile(path: string): Promise<api.FileResponse>;
  readFileRaw(path: string): Promise<string>;
}

/** The route that reads a subject's files, and so its relative assets: the one its identity names. */
export function subjectReader(subject: CanvasSubject): ArtifactPreviewReader | null {
  switch (subject.kind) {
    case "draft_file":
      return {
        readFile: (path) => api.readSandboxCandidateFile(subject.sessionId, subject.candidateId, path),
        readFileRaw: (path) => api.readSandboxCandidateFileRaw(subject.sessionId, subject.candidateId, path),
      };
    case "execution_artifact":
      return {
        readFile: (path) => api.readExecutionArtifact(subject.sessionId, subject.executionId, path),
        readFileRaw: (path) => api.readExecutionArtifactRaw(subject.sessionId, subject.executionId, path),
      };
    case "file":
    case "inline":
      return api;
    case "live_server":
      return null;
  }
}

/** Resolve assets against the file, never the client app's origin. Reads still
 * cross the authenticated, workspace-confined filesystem endpoint. */
export async function artifactPreviewHtml(path: string, html: string, reader: ArtifactPreviewReader = api): Promise<string> {
  const document = new DOMParser().parseFromString(html, "text/html");
  document.querySelectorAll("base").forEach((element) => element.remove());
  const resolve = (value: string, parent = path) => {
    if (!value || /^(?:[a-z][a-z\d+.-]*:|\/\/|#)/i.test(value)) return null;
    const clean = decodeURIComponent(value.split(/[?#]/)[0]);
    if (clean.startsWith("/") || clean.split("/").includes("..") || clean.includes("\\")) {
      throw new Error("Preview assets must be saved beside the preview or in its subfolders.");
    }
    return clean.startsWith("/") ? clean : `${parent.slice(0, parent.lastIndexOf("/") + 1)}${clean}`;
  };
  const dataUrl = async (file: string) => {
    const url = await reader.readFileRaw(file);
    try {
      const blob = await (await fetch(url)).blob();
      return await new Promise<string>((resolve, reject) => {
        const reader = new FileReader();
        reader.onload = () => resolve(String(reader.result));
        reader.onerror = () => reject(new Error(`Unable to load asset: ${file}`));
        reader.readAsDataURL(blob);
      });
    } finally { URL.revokeObjectURL(url); }
  };
  const stylesheet = async (css: string, parent: string) => {
    const matches = [...css.matchAll(/url\(\s*(['"]?)(.*?)\1\s*\)/gi)];
    for (const match of matches) {
      const file = resolve(match[2], parent);
      if (file) css = css.replace(match[0], `url("${await dataUrl(file)}")`);
    }
    return css;
  };
  for (const element of document.querySelectorAll("[src], link[rel='stylesheet'], style, [style]")) {
    if (element.hasAttribute("style")) element.setAttribute("style", await stylesheet(element.getAttribute("style")!, path));
    if (element.tagName === "STYLE") element.textContent = await stylesheet(element.textContent ?? "", path);
    const attr = element.tagName === "LINK" ? "href" : "src";
    const file = resolve(element.getAttribute(attr) ?? "");
    if (!file) continue;
    if (element.tagName === "LINK" || element.tagName === "SCRIPT") {
      const response = await reader.readFile(file);
      if (response.content === undefined) throw new Error(`Unable to read preview asset: ${file}`);
      if (element.tagName === "LINK") {
        const style = document.createElement("style");
        style.textContent = await stylesheet(response.content, file);
        element.replaceWith(style);
      } else {
        element.removeAttribute("src");
        element.textContent = response.content.replace(/<\/script/gi, "<\\/script");
      }
    } else {
      element.setAttribute(attr, await dataUrl(file));
      element.removeAttribute("srcset");
    }
  }
  return sandboxedSrcdoc(`<!doctype html>${document.documentElement.outerHTML}`);
}
