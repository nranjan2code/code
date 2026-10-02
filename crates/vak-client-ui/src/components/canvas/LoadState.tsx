import { Show, type JSX } from "solid-js";
import Icon from "../Icon";
import type { Loader } from "./createLoader";

/** Spinner while a viewer reads, the reason and a retry when it cannot, otherwise its content. */
export default function LoadState<T>(props: { loader: Loader<T>; extra?: JSX.Element; stable?: boolean; children: (data: T) => JSX.Element }) {
  const liveData = new Proxy({} as object, {
    get: (_target, property) => {
      const value = props.loader.data();
      return value && typeof value === "object" ? Reflect.get(value, property) : undefined;
    },
  });
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
        {/* Keep the viewer subtree mounted for same-source refreshes. Its stable
            proxy resolves each property through the live loader signal so the
            new snapshot updates the page without remounting visible rows. */}
        <Show when={props.stable && props.loader.data()}>{props.children(liveData as T)}</Show>
        <Show when={!props.stable && props.loader.data()} keyed>{(data) => props.children(data)}</Show>
      </Show>
      <Show when={props.loader.loading() && !!props.loader.data()}>
        <span class="artifact-canvas-refreshing" role="status">Refreshing…</span>
      </Show>
    </>
  );
}
