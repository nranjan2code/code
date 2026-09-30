import { For, Show, createMemo, createSignal } from "solid-js";
import type { MailCalendarEventPreview } from "../api";

type View = "agenda" | "day" | "week";

interface Props {
  events: MailCalendarEventPreview[];
  from: string;
  to: string;
  conflicts: Set<string>;
  onDraftUpdate?: (event: MailCalendarEventPreview) => void;
}

const dateKey = (date: Date) => `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
const parseDateKey = (key: string) => {
  const [year, month, day] = key.split("-").map(Number);
  return new Date(year, month - 1, day);
};
const dayLabel = (key: string) => parseDateKey(key).toLocaleDateString(undefined, { weekday: "short", month: "short", day: "numeric" });
const startOfDay = (key: string) => parseDateKey(key).getTime();
const addDays = (key: string, amount: number) => {
  const date = parseDateKey(key);
  date.setDate(date.getDate() + amount);
  return dateKey(date);
};
const wallMinutes = (date: Date) => date.getHours() * 60 + date.getMinutes() + date.getSeconds() / 60;

export function MailCalendarAgenda(props: Props) {
  const [view, setView] = createSignal<View>("agenda");
  const days = createMemo(() => {
    const from = parseDateKey(props.from);
    const to = parseDateKey(props.to);
    const result: string[] = [];
    for (const day = new Date(from); day <= to && result.length < 31; day.setDate(day.getDate() + 1)) result.push(dateKey(day));
    return result;
  });
  const shownDays = createMemo(() => view() === "day" ? days().slice(0, 1) : view() === "week" ? days().slice(0, 7) : days());
  const eventsByDay = (key: string) => props.events.filter((event) => {
    if (event.all_day) {
      const start = event.starts_on;
      const end = event.ends_on ?? (start ? addDays(start, 1) : null);
      return !!start && !!end && start < addDays(key, 1) && end > key;
    }
    if (!event.starts_at || !event.ends_at) return false;
    const dayStart = startOfDay(key);
    const dayEnd = startOfDay(addDays(key, 1));
    const start = new Date(event.starts_at).getTime();
    const end = new Date(event.ends_at).getTime();
    return Number.isFinite(start) && Number.isFinite(end) && start < dayEnd && end > dayStart;
  }).sort((a, b) => {
    if (a.all_day !== b.all_day) return a.all_day ? -1 : 1;
    return (a.starts_at ? new Date(a.starts_at).getTime() : startOfDay(a.starts_on ?? key)) - (b.starts_at ? new Date(b.starts_at).getTime() : startOfDay(b.starts_on ?? key));
  });
  const labelTime = (event: MailCalendarEventPreview, key: string) => {
    if (event.all_day) return "All day";
    if (!event.starts_at || !event.ends_at) return "Time unavailable";
    const from = new Date(event.starts_at);
    const to = new Date(event.ends_at);
    const dayStart = parseDateKey(key);
    const dayEnd = new Date(dayStart);
    dayEnd.setDate(dayEnd.getDate() + 1);
    const clippedStart = from < dayStart ? dayStart : from;
    const clippedEnd = to > dayEnd ? dayEnd : to;
    const time = (date: Date) => date.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
    return `${time(clippedStart)}–${time(clippedEnd)}`;
  };
  const eligibleUpdate = (event: MailCalendarEventPreview) => !!props.onDraftUpdate && !!event.version && !event.private && !event.all_day && !event.recurring && event.attendee_count === 0 && !!event.starts_at && !!event.ends_at;
  const renderEvent = (event: MailCalendarEventPreview, key: string) => <article class={`mail-calendar-event${props.conflicts.has(event.provider_id) ? " is-conflict" : ""}`}>
    <div class="mail-calendar-event-time">{labelTime(event, key)}</div>
    <div class="mail-calendar-event-content"><strong>{event.title}</strong><Show when={props.conflicts.has(event.provider_id)}><small>Overlaps another event in this preview.</small></Show><Show when={event.location}><span>{event.location}</span></Show><Show when={event.description}><p>{event.description}</p></Show><Show when={eligibleUpdate(event)}><button class="settings-button" onClick={() => props.onDraftUpdate?.(event)}>Draft update</button></Show></div>
  </article>;

  return <section class="mail-calendar-agenda" aria-label="Calendar events">
    <div class="mail-calendar-view-switch" role="group" aria-label="Calendar view">
      <For each={[["agenda", "Agenda"], ["day", "Day"], ["week", "Week"]] as const}>{([value, label]) => <button type="button" class="settings-button" aria-pressed={view() === value} onClick={() => setView(value)}>{label}</button>}</For>
      <Show when={view() !== "agenda"}><span>{view() === "day" ? dayLabel(shownDays()[0] ?? props.from) : `Week from ${dayLabel(shownDays()[0] ?? props.from)}`}</span></Show>
    </div>
    <Show when={view() === "agenda"}>
      <div class="mail-calendar-agenda-list"><For each={shownDays()}>{(key) => <section class="mail-calendar-agenda-day"><h3>{dayLabel(key)}</h3><Show when={eventsByDay(key).length > 0} fallback={<p class="settings-hint">No events</p>}><div class="mail-calendar-agenda-events"><For each={eventsByDay(key)}>{(event) => renderEvent(event, key)}</For></div></Show></section>}</For></div>
    </Show>
    <Show when={view() === "day" || view() === "week"}>
      <div class={`mail-calendar-time-grid ${view() === "day" ? "is-day" : "is-week"}`}>
        <div class="mail-calendar-time-labels"><div class="mail-calendar-all-day-label">All day</div><For each={Array.from({ length: 24 }, (_, hour) => hour)}>{(hour) => <div>{new Date(2000, 0, 1, hour).toLocaleTimeString([], { hour: "numeric" })}</div>}</For></div>
        <For each={shownDays()}>{(key) => <section class="mail-calendar-time-day" aria-label={dayLabel(key)}>
          <h3>{dayLabel(key)}</h3><div class="mail-calendar-all-day-lane"><For each={eventsByDay(key).filter((event) => event.all_day)}>{(event) => <div class="mail-calendar-all-day-event" title={event.title}>{event.title}</div>}</For></div>
          <div class="mail-calendar-hour-lanes"><For each={Array.from({ length: 24 }, (_, hour) => hour)}>{() => <div />}</For>
            <For each={eventsByDay(key).filter((event) => !event.all_day && event.starts_at && event.ends_at)}>{(event) => {
              const start = new Date(event.starts_at!).getTime();
              const end = new Date(event.ends_at!).getTime();
              const localStart = new Date(start);
              const localEnd = new Date(end);
              const dayStart = parseDateKey(key);
              const dayEnd = parseDateKey(addDays(key, 1));
              const clippedStart = localStart < dayStart ? dayStart : localStart;
              const clippedEnd = localEnd > dayEnd ? dayEnd : localEnd;
              const top = wallMinutes(clippedStart);
              const bottom = clippedEnd.getTime() === dayEnd.getTime() ? 1440 : wallMinutes(clippedEnd);
              const height = Math.max(30, bottom - top);
              return <article class={`mail-calendar-grid-event${props.conflicts.has(event.provider_id) ? " is-conflict" : ""}`} style={{ top: `${top / 1440 * 100}%`, height: `${height / 1440 * 100}%` }} title={`${event.title} · ${labelTime(event, key)}`}><strong>{event.title}</strong><span>{labelTime(event, key)}</span><Show when={eligibleUpdate(event)}><button class="settings-button" onClick={() => props.onDraftUpdate?.(event)}>Draft update</button></Show></article>;
            }}</For>
          </div>
        </section>}</For>
      </div>
    </Show>
  </section>;
}
