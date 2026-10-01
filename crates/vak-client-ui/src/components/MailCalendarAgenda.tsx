import { For, Show, createMemo, createSignal } from "solid-js";
import type { MailCalendarEventPreview } from "../api";

type View = "agenda" | "day" | "week";
type EventLayout = { lane: number; laneCount: number };

interface Props {
  events: MailCalendarEventPreview[];
  from: string;
  to: string;
  conflicts: Set<string>;
  primaryAccountId?: string;
  onDraftUpdate?: (event: MailCalendarEventPreview) => void;
  onDraftCancel?: (event: MailCalendarEventPreview) => void;
  onSelect?: (event: MailCalendarEventPreview) => void;
  initialView?: View;
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
const eventLayoutKey = (day: string, event: MailCalendarEventPreview) => `${day}::${event.account_id ?? ""}::${event.provider_id}`;

export function MailCalendarAgenda(props: Props) {
  const [view, setView] = createSignal<View>(props.initialView ?? "agenda");
  const days = createMemo(() => {
    const from = parseDateKey(props.from);
    const to = parseDateKey(props.to);
    const result: string[] = [];
    for (const day = new Date(from); day <= to && result.length < 31; day.setDate(day.getDate() + 1)) result.push(dateKey(day));
    return result;
  });
  const shownDays = createMemo(() => view() === "day" ? days().slice(0, 1) : view() === "week" ? days().slice(0, 7) : days());
  const sourceEvents = createMemo(() => props.events);
  const eventsByDay = (key: string) => sourceEvents().filter((event) => {
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
  const visibleHours = createMemo(() => {
    const timed = shownDays().flatMap((key) => eventsByDay(key).filter((event) => !event.all_day && event.starts_at && event.ends_at));
    if (!timed.length) return { start: 8, end: 18 };
    const first = Math.min(...timed.map((event) => wallMinutes(new Date(event.starts_at!))));
    const last = Math.max(...timed.map((event) => wallMinutes(new Date(event.ends_at!))));
    return { start: Math.max(0, Math.floor(first / 60) - 1), end: Math.min(24, Math.floor((last - 1) / 60) + 2) };
  });
  const eventLayouts = createMemo(() => {
    const layouts = new Map<string, EventLayout>();
    for (const key of shownDays()) {
      let group: MailCalendarEventPreview[] = [];
      let groupEnd = -Infinity;
      const placeGroup = () => {
        if (!group.length) return;
        const laneEnds: number[] = [];
        const placements: Array<{ key: string; lane: number }> = [];
        for (const event of group) {
          const start = new Date(event.starts_at!).getTime();
          const end = new Date(event.ends_at!).getTime();
          let lane = laneEnds.findIndex((laneEnd) => laneEnd <= start);
          if (lane < 0) lane = laneEnds.length;
          laneEnds[lane] = end;
          placements.push({ key: eventLayoutKey(key, event), lane });
        }
        for (const placement of placements) layouts.set(placement.key, { lane: placement.lane, laneCount: laneEnds.length });
        group = [];
        groupEnd = -Infinity;
      };
      const timed = eventsByDay(key).filter((event) => !event.all_day && event.starts_at && event.ends_at);
      for (const event of timed) {
        const start = new Date(event.starts_at!).getTime();
        const end = new Date(event.ends_at!).getTime();
        if (start >= groupEnd) placeGroup();
        group.push(event);
        groupEnd = Math.max(groupEnd, end);
      }
      placeGroup();
    }
    return layouts;
  });
  const maxLaneCount = () => Math.max(1, ...Array.from(eventLayouts().values(), (layout) => layout.laneCount));
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
  const conflictKey = (event: MailCalendarEventPreview) => event.account_id ? `${event.account_id}::${event.provider_id}` : event.provider_id;
  const eligibleUpdate = (event: MailCalendarEventPreview) => !!props.onDraftUpdate && (!event.account_id || event.account_id === props.primaryAccountId) && !!event.version && !event.private && !event.all_day && !event.recurring && event.attendee_count === 0 && !!event.starts_at && !!event.ends_at;
  const eligibleCancel = (event: MailCalendarEventPreview) => !!props.onDraftCancel && (!event.account_id || event.account_id === props.primaryAccountId) && event.can_cancel === true && !!event.version;
  const selectEvent = (event: MailCalendarEventPreview) => props.onSelect?.(event);
  const renderEvent = (event: MailCalendarEventPreview, key: string) => <article class={`mail-calendar-event${props.conflicts.has(conflictKey(event)) ? " is-conflict" : ""}${props.onSelect ? " is-selectable" : ""}`} role={props.onSelect ? "button" : undefined} tabIndex={props.onSelect ? 0 : undefined} onClick={() => selectEvent(event)} onKeyDown={(e) => { if (props.onSelect && (e.key === "Enter" || e.key === " ")) { e.preventDefault(); selectEvent(event); } }}>
    <div class="mail-calendar-event-time">{labelTime(event, key)}</div>
    <div class="mail-calendar-event-content"><strong>{event.title}</strong><Show when={event.account_name}><small>{event.account_name}</small></Show><Show when={props.conflicts.has(conflictKey(event))}><small>Overlaps another event in this preview.</small></Show><Show when={event.location}><span>{event.location}</span></Show><Show when={event.description}><p>{event.description}</p></Show><Show when={eligibleUpdate(event)}><button class="settings-button" onClick={() => props.onDraftUpdate?.(event)}>Draft update</button></Show><Show when={eligibleCancel(event)}><button class="settings-button danger" onClick={() => props.onDraftCancel?.(event)}>Review cancellation</button></Show></div>
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
      <div class={`mail-calendar-time-grid ${view() === "day" ? "is-day" : "is-week"}`} style={{ "--mail-calendar-day-min-width": `${Math.max(680, maxLaneCount() * 144)}px` }}>
        <div class="mail-calendar-time-labels"><div class="mail-calendar-all-day-label">All day</div><For each={Array.from({ length: visibleHours().end - visibleHours().start }, (_, index) => index + visibleHours().start)}>{(hour) => <div>{new Date(2000, 0, 1, hour).toLocaleTimeString([], { hour: "numeric" })}</div>}</For></div>
        <For each={shownDays()}>{(key) => <section class="mail-calendar-time-day" aria-label={dayLabel(key)}>
          <h3>{dayLabel(key)}</h3><div class="mail-calendar-all-day-lane"><For each={eventsByDay(key).filter((event) => event.all_day)}>{(event) => <button type="button" class="mail-calendar-all-day-event" aria-label={`Open ${event.title}`} onClick={() => selectEvent(event)}>{event.title}</button>}</For></div>
          <div class="mail-calendar-hour-lanes" style={{ "--mail-calendar-hours": String(visibleHours().end - visibleHours().start) }}><For each={Array.from({ length: visibleHours().end - visibleHours().start }, (_, hour) => hour)}>{() => <div />}</For>
            <For each={eventsByDay(key).filter((event) => !event.all_day && event.starts_at && event.ends_at)}>{(event) => {
              const start = new Date(event.starts_at!).getTime();
              const end = new Date(event.ends_at!).getTime();
              const localStart = new Date(start);
              const localEnd = new Date(end);
              const dayStart = parseDateKey(key);
              const dayEnd = parseDateKey(addDays(key, 1));
              const clippedStart = localStart < dayStart ? dayStart : localStart;
              const clippedEnd = localEnd > dayEnd ? dayEnd : localEnd;
              const visibleStart = visibleHours().start * 60;
              const visibleEnd = visibleHours().end * 60;
              const top = Math.max(visibleStart, wallMinutes(clippedStart));
              const bottom = Math.min(visibleEnd, clippedEnd.getTime() === dayEnd.getTime() ? 1440 : wallMinutes(clippedEnd));
              const range = visibleEnd - visibleStart;
              const layout = eventLayouts().get(eventLayoutKey(key, event)) ?? { lane: 0, laneCount: 1 };
              const height = Math.max(1, bottom - top);
              return <article class={`mail-calendar-grid-event${props.conflicts.has(conflictKey(event)) ? " is-conflict" : ""}${props.onSelect ? " is-selectable" : ""}`} role={props.onSelect ? "button" : undefined} aria-label={props.onSelect ? `${event.title}, ${labelTime(event, key)}, ${event.account_name ?? "Calendar"}` : undefined} tabIndex={props.onSelect ? 0 : undefined} onClick={() => selectEvent(event)} onKeyDown={(e) => { if (props.onSelect && (e.key === "Enter" || e.key === " ")) { e.preventDefault(); selectEvent(event); } }} style={{ top: `${(top - visibleStart) / range * 100}%`, height: `${height / range * 100}%`, left: `calc(${layout.lane / layout.laneCount * 100}% + 2px)`, width: `calc(${100 / layout.laneCount}% - 4px)` }} title={`${event.title} · ${labelTime(event, key)} · ${event.account_name ?? "Calendar"}`}><strong>{event.title}</strong><Show when={!props.onSelect && event.account_name}><small>{event.account_name}</small></Show><Show when={eligibleUpdate(event)}><button class="settings-button" onClick={(e) => { e.stopPropagation(); props.onDraftUpdate?.(event); }}>Draft update</button></Show><Show when={eligibleCancel(event)}><button class="settings-button danger" onClick={(e) => { e.stopPropagation(); props.onDraftCancel?.(event); }}>Review cancellation</button></Show></article>;
            }}</For>
          </div>
        </section>}</For>
      </div>
    </Show>
  </section>;
}
