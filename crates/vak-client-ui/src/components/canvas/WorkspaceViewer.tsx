import { Show } from "solid-js";
import InstalledWorkspaceViewer from "./InstalledWorkspaceViewer";
import { Dynamic } from "solid-js/web";
import { CanvasWorkspaceLease } from "./WorkspaceContext";
import { canvasWorkspace } from "../../canvasWorkspaces";
import type { ViewerProps } from "./types";

export default function WorkspaceViewer(props: ViewerProps) {
  const definition = () => props.subject.kind === "workspace" ? canvasWorkspace(props.subject.workspaceType) : undefined;
  return <Show keyed when={definition()} fallback={<CanvasWorkspaceLease><InstalledWorkspaceViewer {...props} /></CanvasWorkspaceLease>}>
    {found => <CanvasWorkspaceLease><Dynamic component={found.component} {...props} /></CanvasWorkspaceLease>}
  </Show>;
}
