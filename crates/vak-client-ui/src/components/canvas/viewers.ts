import type { Component } from "solid-js";
import type { ArtifactDisplayType } from "../../canvasSubject";
import HtmlViewer from "./HtmlViewer";
import { ImageViewer } from "./MediaViewer";
import OfficeViewer from "./OfficeViewer";
import PdfViewer from "./PdfViewer";
import ServerViewer from "./ServerViewer";
import SourceViewer from "./SourceViewer";
import TableViewer from "./TableViewer";
import type { ViewerProps } from "./types";

/** The component that draws each kind of subject; what each can do is in `canvasViewers.ts`. */
export const VIEWERS: Record<ArtifactDisplayType, Component<ViewerProps>> = {
  html: HtmlViewer,
  server: ServerViewer,
  code: SourceViewer,
  table: TableViewer,
  image: ImageViewer,
  pdf: PdfViewer,
  office: OfficeViewer,
};
