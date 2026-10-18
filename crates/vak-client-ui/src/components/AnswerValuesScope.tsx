import { createEffect, onCleanup, Show, type JSX } from "solid-js";
import { AnswerValuesContext, acquireAnswerValuesScope } from "../answerValues";
import type { OutputItem } from "../types";

export const conversationValuesKey = (session: string, turn: string) => JSON.stringify(["conversation", session, turn]);
export const pageValuesKey = (version: string) => JSON.stringify(["page", version]);

/** A view owns only the sources it shows. Other views of the same answer
 * keep their sources and current inputs until their final view closes. */
function MountedScope(props: { scopeKey: string; items: OutputItem[]; children: JSX.Element }) {
  const lease = acquireAnswerValuesScope(props.scopeKey);
  const sources = () => props.items.flatMap((item) => item.provenance?.presentation_id ? [item.provenance.presentation_id] : []);
  lease.reconcile(sources());
  createEffect(() => lease.reconcile(sources()));
  onCleanup(() => lease.release());
  return <AnswerValuesContext.Provider value={lease.scope}>{props.children}</AnswerValuesContext.Provider>;
}

/** Changing the immutable answer identity remounts its context. */
export default function AnswerValuesScope(props: { scopeKey: string; items: OutputItem[]; children: JSX.Element }) {
  return <Show when={props.scopeKey} keyed>{(key) =>
    <MountedScope scopeKey={key} items={props.items}>{props.children}</MountedScope>
  }</Show>;
}
