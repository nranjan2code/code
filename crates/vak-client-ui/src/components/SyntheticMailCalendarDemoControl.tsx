import { Show } from "solid-js";
import { canOfferSyntheticMailCalendar } from "../mailCalendarDemo";

export function SyntheticMailCalendarDemoControl(props: { checked: boolean; onChange: (enabled: boolean) => void }) {
  return <Show when={canOfferSyntheticMailCalendar()}>
    <label class="capability-item"><input data-testid="synthetic-mail-calendar-toggle" type="checkbox" aria-label="Use synthetic demo data" checked={props.checked} onChange={(event) => props.onChange(event.currentTarget.checked)} /><span>Use synthetic demo data</span></label>
    <p class="settings-hint" role="status">{props.checked
      ? "Synthetic mode is on. Sample Google, Microsoft and iCloud accounts, messages and events are generated in this browser. Nothing is read from or written to a provider; provider actions and real routines are disabled. Local drafts stay in this browser."
      : "Try the email and calendar screens with sample accounts. This never needs credentials."}</p>
  </Show>;
}
