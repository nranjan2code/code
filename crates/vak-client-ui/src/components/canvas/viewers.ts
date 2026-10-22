import WorkspaceViewer from "./WorkspaceViewer";
import type { Component } from "solid-js";
import type { ArtifactDisplayType } from "../../canvasSubject";
import HtmlViewer from "./HtmlViewer";
import { ImageViewer } from "./MediaViewer";
import OfficeViewer from "./OfficeViewer";
import PdfViewer from "./PdfViewer";
import ServerViewer from "./ServerViewer";
import SourceViewer from "./SourceViewer";
import AutomationViewer from "./AutomationViewer";
import DailyMailCalendarViewer from "./DailyMailCalendarViewer";
import AnswerViewer from "./AnswerViewer";
import PageViewer from "./PageViewer";
import TableViewer from "./TableViewer";
import type { ViewerProps } from "./types";

/** The component that draws each kind of subject; what each can do is in `canvasViewers.ts`. */
export const VIEWERS: Record<ArtifactDisplayType, Component<ViewerProps>> = {
  workspace: WorkspaceViewer,
  html: HtmlViewer,
  server: ServerViewer,
  code: SourceViewer,
  table: TableViewer,
  image: ImageViewer,
  pdf: PdfViewer,
  office: OfficeViewer,
  automation: AutomationViewer,
  daily_mail_calendar: DailyMailCalendarViewer,
  answer: AnswerViewer,
  page: PageViewer,
};
