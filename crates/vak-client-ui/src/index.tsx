/* @refresh reload */
import { render } from "solid-js/web";
import "./styles.css";
import App from "./App";
import AppErrorBoundary from "./components/AppErrorBoundary";
import SharedConversation from "./components/SharedConversation";

const sharedView = new URLSearchParams(window.location.search).get("shared") === "1";

render(
  () => (
    <AppErrorBoundary>
      {sharedView ? <SharedConversation /> : <App />}
    </AppErrorBoundary>
  ),
  document.getElementById("root")!,
);
