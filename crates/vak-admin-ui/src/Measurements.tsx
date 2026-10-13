/// System › How the models did. Measured from the turns in this install's
/// conversations (doc 86, doc 89): where a model's calls were refused or
/// failed, where a turn needed a correction, and where it stopped at the
/// step limit. Each pattern is a proposal a person accepts or dismisses;
/// nothing changes by itself.

import { createResource, createSignal, For, Show } from "solid-js";
import { api } from "./api";
import { pushToast } from "./store";
import type { LedgerModelMeasure, Proposal, ProposalPattern } from "./types";

type ReviewFilter = "waiting" | "accepted" | "dismissed";

const FILTERS: { value: ReviewFilter; label: string; empty: string }[] = [
  { value: "waiting", label: "Waiting", empty: "Nothing waiting for an answer." },
  { value: "accepted", label: "Accepted", empty: "Nothing accepted yet." },
  { value: "dismissed", label: "Dismissed", empty: "Nothing dismissed." },
];

const REFUSALS: Record<string, string> = {
  unknown: "it is not a tool this turn offered",
  schema: "its arguments did not fit the tool's schema",
  tool: "the tool refused the arguments",
  repeated: "the same call was made again",
};

const FAULTS: Record<string, string> = {
  shape: "the call's shape was wrong",
  target: "what it named was not there or did not work",
  tool: "the tool itself failed",
  limit: "it reached the configured time limit",
};

/** A pattern in plain words: a title and an optional dim detail line. */
function patternWords(pattern: ProposalPattern): { title: string; detail?: string } {
  switch (pattern.kind) {
    case "tool_refused":
      return { title: `"${pattern.tool}" was refused before it ran`, detail: REFUSALS[pattern.refusal] ?? pattern.refusal };
    case "tool_failed":
      return { title: `"${pattern.tool}" ran and failed`, detail: FAULTS[pattern.fault] ?? pattern.fault };
    case "corrected":
      return { title: "Turns needed a correction", detail: `cause: ${pattern.cause.replace(/_/g, " ")}` };
    case "step_limit":
      return { title: "Turns stopped at the step limit" };
  }
}

const isTool = (pattern: ProposalPattern) => pattern.kind === "tool_refused" || pattern.kind === "tool_failed";

function reviewState(proposal: Proposal): ReviewFilter {
  return proposal.review === null ? "waiting" : proposal.review.review;
}

export function Measurements() {
  const [report, { refetch }] = createResource(() => api.ledgerReport());
  const [filter, setFilter] = createSignal<ReviewFilter>("waiting");

  const act = async (label: string, action: () => Promise<unknown>) => {
    try {
      await action();
      pushToast("info", label);
    } catch (error) {
      pushToast("alert", `${error}`);
    }
    await refetch();
  };

  const visible = () => (report()?.proposals ?? []).filter((p) => reviewState(p) === filter());
  const emptyText = () => FILTERS.find((f) => f.value === filter())!.empty;

  return (
    <div class="page">
      <header class="page-header">
        <div>
          <h1>How the models did</h1>
          <p class="dim">Measured from the turns in this install's conversations. Each pattern below is something Vakyartha could change; nothing changes by itself — you decide.</p>
        </div>
      </header>
      <Show when={!report.error} fallback={<p class="dim">The measurements could not be read: {`${report.error}`}</p>}>
        <Show when={report()}>
          {(data) => (
            <>
              <p class="dim">
                {data().turns} turns in {data().sessions} conversations
                <Show when={data().unreadable > 0}>{` · ${data().unreadable} could not be read`}</Show>
              </p>

              <section class="panel">
                <div class="panel-title-row"><div><span class="eyebrow">By model</span><h2>Each model's turns</h2><p class="dim">Step limit: turns that stopped there. Refused: calls stopped before they ran. Failed: calls that ran and failed. Steps and time are typical for one turn.</p></div></div>
                <Show when={data().models.length > 0} fallback={<div class="empty">No finished turns yet.</div>}>
                  <div class="ops-table-wrap">
                    <table class="ops-table">
                      <thead>
                        <tr>
                          <th>Model</th>
                          <th>Turns</th>
                          <th>Answered</th>
                          <th>Step limit</th>
                          <th>Calls</th>
                          <th>Refused</th>
                          <th>Failed</th>
                          <th>Corrections</th>
                          <th>Steps</th>
                          <th>Time</th>
                        </tr>
                      </thead>
                      <tbody>
                        <For each={data().models}>
                          {(m: LedgerModelMeasure) => (
                            <tr>
                              <td class="mono nowrap">{m.model}</td>
                              <td>{m.turns}</td>
                              <td>{m.answered}</td>
                              <td>{m.step_limit}</td>
                              <td>{m.calls}</td>
                              <td>{m.refused_calls}</td>
                              <td>{m.failed_calls}</td>
                              <td>{m.corrections}</td>
                              <td>{m.median_steps === null ? "–" : m.median_steps.toFixed(1)}</td>
                              <td>{m.median_seconds === null ? "–" : `${m.median_seconds.toFixed(1)} s`}</td>
                            </tr>
                          )}
                        </For>
                      </tbody>
                    </table>
                  </div>
                </Show>
              </section>

              <section class="panel">
                <div class="panel-title-row"><div><span class="eyebrow">Patterns to review</span><h2>What might change</h2></div></div>
                <div class="tab-bar" role="group" aria-label="Show patterns">
                  <For each={FILTERS}>
                    {(f) => (
                      <button class="tab-btn" classList={{ active: filter() === f.value }} aria-pressed={filter() === f.value} onClick={() => setFilter(f.value)}>
                        {f.label}
                        <span class="tab-count">{(report()?.proposals ?? []).filter((p) => reviewState(p) === f.value).length}</span>
                      </button>
                    )}
                  </For>
                </div>
                <Show when={visible().length > 0} fallback={<div class="empty">{emptyText()}</div>}>
                  <div class="ops-table-wrap">
                    <table class="ops-table">
                      <thead><tr><th>Pattern</th><th>Answer</th></tr></thead>
                      <tbody>
                        <For each={visible()}>
                          {(proposal) => {
                            const words = patternWords(proposal.pattern);
                            return (
                              <tr>
                                <td>
                                  {words.title}
                                  <Show when={words.detail}><div class="dim">{words.detail}</div></Show>
                                  <div class="dim">
                                    {proposal.count} of {proposal.of} {isTool(proposal.pattern) ? "calls" : "turns"} on <span class="mono nowrap">{proposal.model}</span>
                                  </div>
                                  <details class="run-technical">
                                    <summary>Technical details</summary>
                                    <dl class="ops-detail-grid">
                                      <div><dt>Key</dt><dd class="mono">{proposal.key}</dd></div>
                                      <div><dt>Turns</dt><dd class="mono">{proposal.turns.join(", ")}</dd></div>
                                    </dl>
                                  </details>
                                </td>
                                <td>
                                  <Show
                                    when={proposal.review}
                                    fallback={
                                      <div class="ops-action-row">
                                        <button class="ghost small" onClick={() => void act("Recorded", () => api.reviewProposal(proposal.key, "accepted"))}>Accept</button>
                                        <button class="ghost small" onClick={() => void act("Recorded", () => api.reviewProposal(proposal.key, "dismissed"))}>Dismiss</button>
                                      </div>
                                    }
                                  >
                                    {(review) => (
                                      <>
                                        <span class={`chip ${review().review === "accepted" ? "chip-tone-success" : ""}`}>
                                          {review().review === "accepted" ? "Accepted" : "Dismissed"}
                                        </span>
                                        <div class="dim">{new Date(review().at).toLocaleDateString()}</div>
                                      </>
                                    )}
                                  </Show>
                                </td>
                              </tr>
                            );
                          }}
                        </For>
                      </tbody>
                    </table>
                  </div>
                </Show>
              </section>
            </>
          )}
        </Show>
      </Show>
    </div>
  );
}
