/* @refresh reload */
import { render } from "solid-js/web";
// Priming import: matches the production entry's import order (index.tsx
// imports App first). PresentationRenderer.tsx transitively imports App.tsx,
// and importing it in a different order than the production entry trips a
// pre-existing ESM circular-import TDZ between ChatPane.tsx and
// MarkdownView.tsx. Importing App first here resolves the cycle the same
// way the real app does, without touching production source.
import "../App";
import "../styles.css";
import "./harness.css";
import CardHarness from "./CardHarness";

render(() => <CardHarness />, document.getElementById("root")!);
