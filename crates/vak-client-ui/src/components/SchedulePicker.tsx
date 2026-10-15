import { createSignal, For, Show } from "solid-js";
import type { Repeat } from "../types";
import { technicalDetails } from "../store";

const DAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
type Kind = Repeat["kind"];

/** How a person picks when an automation runs (docs/design/94 §5): never a
 * cron box, except "Custom" behind Show technical details. Times are the
 * owner's zone, which it names. */
export default function SchedulePicker(props: { zone?: string; onPick: (repeat: Repeat) => void; busy?: boolean; label?: string }) {
  const [kind, setKind] = createSignal<Kind>("daily");
  const [time, setTime] = createSignal("09:00");
  const [date, setDate] = createSignal(new Date(Date.now() + 86_400_000).toISOString().slice(0, 10));
  const [days, setDays] = createSignal<number[]>([1]);
  const [count, setCount] = createSignal(30);
  const [cron, setCron] = createSignal("0 9 * * 1-5");
  const repeat = (): Repeat | null => {
    switch (kind()) {
      case "once": return { kind: "once", at: `${date()} ${time()}` };
      case "daily": return { kind: "daily", time: time() };
      case "weekdays": return { kind: "weekdays", time: time() };
      case "weekly": return days().length ? { kind: "weekly", days: days(), time: time() } : null;
      case "every_minutes": return count() >= 1 ? { kind: "every_minutes", minutes: count() } : null;
      case "every_hours": return count() >= 1 ? { kind: "every_hours", hours: count() } : null;
      case "custom": return cron().trim() ? { kind: "custom", cron: cron().trim() } : null;
    }
  };
  const toggle = (day: number) => setDays((current) => current.includes(day) ? current.filter((d) => d !== day) : [...current, day].sort());
  return <div class="schedule-picker">
    <label>When<select aria-label="When it runs" value={kind()} onChange={(event) => { const next = event.currentTarget.value as Kind; setKind(next); if (next === "every_minutes") setCount(30); if (next === "every_hours") setCount(1); }}>
      <option value="once">Once</option>
      <option value="daily">Every day</option>
      <option value="weekdays">Weekdays</option>
      <option value="weekly">Every week on…</option>
      <option value="every_minutes">Every few minutes</option>
      <option value="every_hours">Every few hours</option>
      <Show when={technicalDetails()}><option value="custom">Custom (cron)</option></Show>
    </select></label>
    <Show when={kind() === "once"}><label>Date<input type="date" value={date()} onInput={(event) => setDate(event.currentTarget.value)} /></label></Show>
    <Show when={kind() === "weekly"}><div class="schedule-days" role="group" aria-label="Days"><For each={DAYS}>{(name, day) => <button type="button" class="schedule-day" aria-pressed={days().includes(day())} onClick={() => toggle(day())}>{name}</button>}</For></div></Show>
    <Show when={["once", "daily", "weekdays", "weekly"].includes(kind())}><label>At<input type="time" value={time()} onInput={(event) => setTime(event.currentTarget.value)} /></label></Show>
    <Show when={kind() === "every_minutes" || kind() === "every_hours"}><label>Every<input class="settings-number" type="number" min="1" value={count()} onInput={(event) => setCount(Number(event.currentTarget.value))} /><span>{kind() === "every_minutes" ? "minutes" : "hours"}</span></label></Show>
    <Show when={kind() === "custom"}><label>Cron<input value={cron()} onInput={(event) => setCron(event.currentTarget.value)} /></label></Show>
    <Show when={props.zone}><span class="schedule-zone">Times are {props.zone!.replaceAll("_", " ")}</span></Show>
    <button type="button" class="btn primary" disabled={props.busy || !repeat()} onClick={() => { const picked = repeat(); if (picked) props.onPick(picked); }}>{props.label ?? "Save"}</button>
  </div>;
}
