/// Work › Learned abilities (docs/design/93 §9.6): every ability each Agent
/// made, where it is (that Agent, every Agent, waiting), the version in
/// use and what it reaches; and the skills every Agent finds, with where
/// each comes from. Revoking takes effect at once: the next call to it is
/// refused, and its versions stay in the Library.

import { createResource, For, Show } from "solid-js";
import { api } from "./api";
import { pushToast } from "./store";
import type { CapabilityAbility } from "./types";

export const ABILITY_STATE: Record<CapabilityAbility["state"], { word: string; tone: string }> = {
  working: { word: "Working", tone: "chip-tone-success" },
  failing: { word: "Failing", tone: "chip-tone-danger" },
  offered: { word: "Ready for every Agent", tone: "chip-tone-info" },
  waiting: { word: "Waiting for review", tone: "chip-tone-warning" },
  turned_off: { word: "Turned off", tone: "" },
};

const ORIGIN: Record<string, string> = {
  shared: "Shared",
  project: "This project",
  plugin: "Add-on",
  agent: "Agent-made",
  builtin: "Built in",
};

export function Capabilities() {
  const [inventory, { refetch }] = createResource(() => api.capabilities());
  const revoke = async (name: string, agent?: string) => {
    const whom = agent ? "this Agent" : "every Agent";
    if (!window.confirm(`Take ${name} back from ${whom}? Its next call is refused; its versions stay in the Library.`)) return;
    try {
      await api.revokeCapability(name, agent);
      pushToast("info", `${name} was taken back from ${whom}.`);
    } catch (error) {
      pushToast("alert", error instanceof Error ? error.message : String(error));
    }
    void refetch();
  };
  const current = (ability: CapabilityAbility) => ability.measure.find((version) => version.current);
  const installed = (ability: CapabilityAbility) => ability.state === "working" || ability.state === "failing" || ability.state === "offered";
  return (
    <div class="page">
      <header class="page-header">
        <div>
          <h1>Learned abilities</h1>
          <p class="dim">What each Agent taught itself and the skills every Agent finds. Taking one back works at once; its versions stay in the Library.</p>
        </div>
      </header>
      <Show when={!inventory.error} fallback={<p class="dim">The abilities could not be read: {`${inventory.error}`}</p>}>
        <Show when={inventory()} fallback={<div class="empty">Loading…</div>}>
          {(data) => (
            <>
              <section class="panel">
                <div class="panel-title-row"><div><span class="eyebrow">By Agent</span><h2>Learned by your Agents</h2></div></div>
                <Show when={data().abilities.length > 0} fallback={<div class="empty">No Agent has learned an ability yet.</div>}>
                  <div class="ops-table-wrap">
                    <table class="ops-table">
                      <thead>
                        <tr><th>Ability</th><th>Agent</th><th>State</th><th>Where</th><th>Version</th><th>What it reaches</th><th /></tr>
                      </thead>
                      <tbody>
                        <For each={data().abilities}>
                          {(ability) => (
                            <tr>
                              <td><strong>{ability.name}</strong><Show when={ability.description}><div class="dim">{ability.description}</div></Show><Show when={ability.why}><div class="dim">Waiting because {ability.why}.</div></Show></td>
                              <td>{ability.agent_name}</td>
                              <td><span class={`chip ${ABILITY_STATE[ability.state].tone}`}>{ABILITY_STATE[ability.state].word}</span></td>
                              <td>{ability.shared ? "Every Agent" : ability.state === "waiting" ? "Waiting" : installed(ability) ? "This Agent" : "Nowhere"}</td>
                              <td>{current(ability)?.number ?? "–"}<Show when={ability.versions > 0}><span class="dim"> of {ability.versions}</span></Show></td>
                              <td><For each={ability.reaches}>{(line) => <div class="dim">{line}</div>}</For></td>
                              <td>
                                <div class="ops-action-row">
                                  <Show when={installed(ability)}>
                                    <button class="ghost small" onClick={() => void revoke(ability.name, ability.agent)}>Take back</button>
                                  </Show>
                                  <Show when={ability.shared}>
                                    <button class="ghost small" onClick={() => void revoke(ability.name)}>Take back from every Agent</button>
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
              </section>
              <section class="panel">
                <div class="panel-title-row"><div><span class="eyebrow">Every Agent</span><h2>Skills every Agent finds</h2></div></div>
                <Show when={data().skills.length > 0} fallback={<div class="empty">No skills are installed.</div>}>
                  <div class="ops-table-wrap">
                    <table class="ops-table">
                      <thead><tr><th>Skill</th><th>From</th><th>Made by</th><th>What it reaches</th></tr></thead>
                      <tbody>
                        <For each={data().skills}>
                          {(skill) => (
                            <tr>
                              <td><strong>{skill.name}</strong><div class="dim">{skill.description}</div></td>
                              <td>{ORIGIN[skill.origin ?? ""] ?? skill.origin ?? "–"}</td>
                              <td>{skill.made_by ?? "–"}</td>
                              <td><Show when={skill.reaches.length > 0} fallback={<span class="dim">Instructions only</span>}><For each={skill.reaches}>{(line) => <div class="dim">{line}</div>}</For></Show></td>
                            </tr>
                          )}
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

/// System › How the models did, by ability version: each installed
/// version's calls and failures, and what R2 says of it (doc 93 §9.6).
export function AbilityVersionsPanel() {
  const [inventory] = createResource(() => api.capabilities());
  const rows = () => (inventory()?.abilities ?? []).flatMap((ability) => ability.measure.map((version) => ({ ability, version })));
  const VERDICT: Record<string, string> = { offer: "Would offer to every Agent", withdraw: "Would turn off", keep: "–" };
  return (
    <section class="panel">
      <div class="panel-title-row"><div><span class="eyebrow">By ability version</span><h2>Each learned ability's versions</h2><p class="dim">Calls made while a version was installed, how many failed, whether its tests passed, and what the promotion rule would do with it now.</p></div></div>
      <Show when={!inventory.error} fallback={<p class="dim">The abilities could not be read: {`${inventory.error}`}</p>}>
        <Show when={rows().length > 0} fallback={<div class="empty">{inventory.loading ? "Loading…" : "No learned ability has an installed version yet."}</div>}>
          <div class="ops-table-wrap">
            <table class="ops-table">
              <thead><tr><th>Ability</th><th>Agent</th><th>Version</th><th>Calls</th><th>Failed</th><th>Tests</th><th>Worked elsewhere</th><th>Promotion rule</th></tr></thead>
              <tbody>
                <For each={rows()}>
                  {({ ability, version }) => (
                    <tr>
                      <td><strong>{ability.name}</strong><Show when={version.current}><span class="dim"> · in use</span></Show></td>
                      <td>{ability.agent_name}</td>
                      <td>{version.number ?? "–"}</td>
                      <td>{version.calls}</td>
                      <td>{version.failed}</td>
                      <td>{version.passed ? "Passed" : "Failed"}</td>
                      <td>{version.succeeded_elsewhere ? "Yes" : "No"}</td>
                      <td>{version.offered ? "Offered" : VERDICT[version.verdict] ?? version.verdict}</td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </div>
        </Show>
      </Show>
    </section>
  );
}
