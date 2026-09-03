/* @refresh reload */
import { render } from "solid-js/web";
import "./styles.css";
import App from "./App";
import AppErrorBoundary from "./components/AppErrorBoundary";

render(
  () => (
    <AppErrorBoundary>
      <App />
    </AppErrorBoundary>
  ),
  document.getElementById("root")!,
);
