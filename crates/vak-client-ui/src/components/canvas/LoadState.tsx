import { Show, type JSX } from "solid-js";
import Icon from "../Icon";
import { technicalDetails } from "../../store";
import type { Loader } from "./createLoader";

/**
 * A spinner while a viewer reads (`label` says what is being opened), the
 * reason and Try again when it cannot, otherwise its content. A reload keeps
 * the content in place with a quiet "Refreshing…" until the new one is ready.
 */
export default function LoadState<T>(props: { loader: Loader<T>; label?: string; extra?: JSX.Element; stable?: boolean; children: (data: T) => JSX.Element }) {
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
            <span>{props.label ?? "Opening…"}</span>
          </div>
        </Show>
      </Show>
      <Show when={props.loader.error()}>
        <div class="artifact-canvas-error" role="alert">
          <Icon name="warning" size={16} />
          <span>
            {props.loader.error()}
            <Show when={technicalDetails() && props.loader.detail() !== props.loader.error() && props.loader.detail()}>
              {(detail) => <small class="artifact-canvas-error-detail">{detail()}</small>}
            </Show>
          </span>
          {props.extra}
          <button type="button" class="artifact-canvas-btn" onClick={props.loader.reload}>Try again</button>
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
