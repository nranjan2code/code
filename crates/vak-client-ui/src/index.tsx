/* @refresh reload */
import { render } from "solid-js/web";
import "./styles.css";
import App from "./App";
import AppErrorBoundary from "./components/AppErrorBoundary";
import SharedConversation from "./components/SharedConversation";
import SharedArtifact from "./components/SharedArtifact";

const shared = new URLSearchParams(window.location.search).get("shared");

render(
  () => (
    <AppErrorBoundary>
      {shared === "1" ? <SharedConversation /> : shared === "artifact" ? <SharedArtifact /> : <App />}
    </AppErrorBoundary>
  ),
  document.getElementById("root")!,
);
