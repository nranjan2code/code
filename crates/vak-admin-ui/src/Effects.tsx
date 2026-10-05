/// Actions (plan M4.5): what Vakyartha did outside itself, today a message
/// sent to a channel. Each is recorded before it is sent. One whose outcome
/// nobody can prove reads "Not sure it was sent" and waits for a person:
/// send it again, or say whether it arrived. It is never sent again by itself.

import { createResource, For, Show } from "solid-js";
import { api } from "./api";
import { pushToast } from "./store";
import type { EffectRecord, EffectStatus } from "./types";

export const EFFECT_WORDS: Record<EffectStatus, string> = {
  held: "Waiting for the digest",
  queued: "Sending",
  sending: "Sending",
  retrying: "Sending",
  sent: "Sent",
  failed: "Didn't send",
  unknown: "Not sure it was sent",
  superseded: "Sent again",
};

const EFFECT_TONE: Record<EffectStatus, string> = {
  held: "",
  queued: "chip-tone-info",
  sending: "chip-tone-info",
  retrying: "chip-tone-warning",
  sent: "chip-tone-success",
  failed: "chip-tone-danger",
  unknown: "chip-tone-warning",
  superseded: "",
};

const SURFACES: Record<string, string> = {
  telegram: "Telegram",
  discord: "Discord",
  slack: "Slack",
  webhook: "a webhook",
  log: "the delivery log",
};

/** What an action did, in the words a person would use. */
export function effectWhat(effect: EffectRecord): string {
  return `Sent to ${SURFACES[effect.kind.surface] ?? effect.kind.surface}`;
}

const canResend = (status: EffectStatus) => ["queued", "retrying", "failed", "unknown"].includes(status);
const canSettle = (status: EffectStatus) => ["retrying", "failed", "unknown"].includes(status);

export function EffectList(props: { effects: EffectRecord[]; onChanged?: () => void | Promise<unknown>; empty?: string }) {
  const act = async (label: string, action: () => Promise<unknown>) => {
    try {
      await action();
      pushToast("info", label);
    } catch (error) {
      pushToast("alert", `${error}`);
    }
    await props.onChanged?.();
  };
  return (
    <Show when={props.effects.length > 0} fallback={<div class="empty">{props.empty ?? "Nothing was sent."}</div>}>
      <div class="ops-table-wrap">
        <table class="ops-table">
          <thead><tr><th>Action</th><th>Status</th><th>When</th><th /></tr></thead>
          <tbody>
            <For each={props.effects}>
              {(effect) => (
                <tr>
                  <td>
                    {effectWhat(effect)}
                    <details class="run-technical">
                      <summary>Technical details</summary>
                      <dl class="ops-detail-grid">
                        <div><dt>Effect id</dt><dd class="mono">{effect.id}</dd></div>
                        <div><dt>Target</dt><dd class="mono">{effect.target}</dd></div>
                        <div><dt>Key</dt><dd class="mono">{effect.idempotency_key}</dd></div>
                        <div><dt>Attempts</dt><dd>{effect.attempts}</dd></div>
                        <Show when={effect.receipt?.provider_id}><div><dt>Provider id</dt><dd class="mono">{effect.receipt!.provider_id}</dd></div></Show>
                        <Show when={effect.supersedes}><div><dt>Replaces</dt><dd class="mono">{effect.supersedes}</dd></div></Show>
                        <Show when={effect.run}><div><dt>Run</dt><dd><a class="mono" href={`#/runs/${encodeURIComponent(effect.run!)}`}>{effect.run}</a></dd></div></Show>
                      </dl>
                    </details>
                  </td>
                  <td>
                    <span class={`chip ${EFFECT_TONE[effect.status]}`}>{EFFECT_WORDS[effect.status]}</span>
                    <Show when={effect.reason && effect.status !== "sent"}><p class="ops-error">{effect.reason}</p></Show>
                  </td>
                  <td>{new Date(effect.updated_at).toLocaleString()}</td>
                  <td>
                    <div class="ops-action-row">
                      <Show when={canResend(effect.status)}>
                        <button class="ghost small" onClick={() => void act("Sent again", () => api.resendEffect(effect.id))}>Send again</button>
                      </Show>
                      <Show when={canSettle(effect.status)}>
                        <button class="ghost small" onClick={() => void act("Marked as sent", () => api.reconcileEffect(effect.id, "sent"))}>It arrived</button>
                        <button class="ghost small" onClick={() => void act("Marked as not sent", () => api.reconcileEffect(effect.id, "not_sent"))}>It didn't arrive</button>
                      </Show>
                    </div>
                  </td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </div>
    </Show>
  );
}

/** The actions one run took. */
export function RunEffects(props: { run: string }) {
  const [effects, { refetch }] = createResource(() => props.run, (run) => api.effects({ run }));
  return (
    <section class="panel">
      <div class="panel-title-row"><div><span class="eyebrow">Actions</span><h2>What it sent</h2></div></div>
      <Show when={!effects.error} fallback={<p class="dim">Its actions could not be read: {`${effects.error}`}</p>}>
        <EffectList effects={effects()?.effects ?? []} onChanged={() => void refetch()} empty="This run sent nothing outside Vakyartha." />
      </Show>
    </section>
  );
}
