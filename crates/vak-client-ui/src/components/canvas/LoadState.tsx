import { Show, type JSX } from "solid-js";
import Icon from "../Icon";
import type { Loader } from "./createLoader";

/** Spinner while a viewer reads, the reason and a retry when it cannot, otherwise its content. */
export default function LoadState<T>(props: { loader: Loader<T>; extra?: JSX.Element; children: (data: T) => JSX.Element }) {
  return (
    <>
      <Show when={props.loader.loading()}>
        <Show when={!props.loader.data()}>
          <div class="artifact-canvas-loading" role="status">
            <div class="artifact-canvas-spinner" />
            <span>Preparing preview…</span>
          </div>
        </Show>
      </Show>
      <Show when={props.loader.error()}>
        <div class="artifact-canvas-error" role="alert">
          <Icon name="warning" size={16} />
          <span>{props.loader.error()}</span>
          {props.extra}
          <button type="button" class="artifact-canvas-btn" onClick={props.loader.reload}>Retry</button>
        </div>
      </Show>
      <Show when={!props.loader.loading() && !props.loader.error() || !!props.loader.data()}>
        <Show when={props.loader.data()} keyed>{(data) => props.children(data as T)}</Show>
      </Show>
      <Show when={props.loader.loading() && !!props.loader.data()}>
        <span class="artifact-canvas-refreshing" role="status">Refreshing…</span>
      </Show>
    </>
  );
}
