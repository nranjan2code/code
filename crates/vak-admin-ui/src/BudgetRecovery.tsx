import { createResource, createSignal, For, Show } from "solid-js";
import { api } from "./api";
import type { PendingCharge } from "./types";

function Charge(props: { charge: PendingCharge; saved: () => void }) {
  const [amount, setAmount] = createSignal("");
  const [verified, setVerified] = createSignal(false);
  const [saving, setSaving] = createSignal(false);
  const [error, setError] = createSignal("");
  const actual = () => Number(amount());
  const valid = () => amount().trim() !== "" && Number.isFinite(actual()) && actual() >= 0 && verified();
  const save = async (event: SubmitEvent) => {
    event.preventDefault();
    if (!valid() || saving()) return;
    setSaving(true);
    setError("");
    try {
      await api.reconcileCost(props.charge.id, actual());
      props.saved();
    } catch {
      setError("The charge was not recorded. Refresh the list and check that the call has finished before trying again.");
    } finally {
      setSaving(false);
    }
  };
  return <form class="panel" onSubmit={save}>
    <h3>{props.charge.provider || "Provider call"}{props.charge.model ? ` · ${props.charge.model}` : ""}</h3>
    <p class="dim">{new Date(props.charge.created_at).toLocaleString()} · {props.charge.estimate_usd == null ? "Reserved amount unknown" : `$${props.charge.estimate_usd.toFixed(4)} reserved`}</p>
    <label>Verified charge in USD
      <input type="number" min="0" step="any" required value={amount()} disabled={saving()} onInput={(event) => setAmount(event.currentTarget.value)} />
    </label>
    <label><input type="checkbox" checked={verified()} disabled={saving()} onChange={(event) => setVerified(event.currentTarget.checked)} /> I checked this call’s final charge with the provider.</label>
    <p class="dim">This records the final cost and replaces this reservation. It does not retry the call.</p>
    <Show when={error()}><p role="alert">{error()}</p></Show>
    <button type="submit" disabled={!valid() || saving()}>{saving() ? "Recording…" : "Record verified cost"}</button>
  </form>;
}

export default function BudgetRecovery(props: { onReconciled: () => void }) {
  const [pending, { refetch }] = createResource(() => api.pendingCosts());
  const refresh = () => { void Promise.resolve(refetch()).catch(() => undefined); };
  const saved = () => { refresh(); props.onReconciled(); };
  return <section class="panel" aria-label="Reserved provider costs">
    <div class="panel-title-row"><h2>Reserved provider costs</h2><button onClick={refresh}>Refresh</button></div>
    <p class="dim">Calls without a recorded outcome keep their cost reserved. Check provider billing before recording a final charge; uncertain charges stay reserved.</p>
    <Show when={pending.error}><p role="alert">Reserved costs could not be verified. Their budget reservations remain in place.</p></Show>
    <Show when={pending.loading && !pending()}><p>Checking reserved costs…</p></Show>
    <Show when={pending()}>{(value) => <>
      <Show when={value().unaddressable_unknown_calls > 0}><p role="alert">{value().unaddressable_unknown_calls} older calls have unknown cost and no verifiable reservation. They remain unknown and cannot be corrected here.</p></Show>
      <Show when={value().reservations.length === 0 && value().unaddressable_unknown_calls === 0}><p>No unresolved provider costs.</p></Show>
      <For each={value().reservations}>{(charge) => <Charge charge={charge} saved={saved} />}</For>
    </>}</Show>
  </section>;
}
