import { For, Show, createSignal, onCleanup, onMount } from "solid-js";
import * as api from "../../api";
import { openMailCalendarCitation, setPendingSettingsPage, setSettingsOpen } from "../../store";
import { MailCalendarAgenda } from "../MailCalendarAgenda";
import { createLoader } from "./createLoader";
import LoadState from "./LoadState";
import type { ViewerProps } from "./types";

const dateKey = (date: Date) => `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
const tomorrow = (date: Date) => { const next = new Date(date); next.setDate(next.getDate() + 1); return dateKey(next); };

type DailyAccount = {
  account: api.MailCalendarAccount;
  messages: api.MailCalendarMailPreview[];
  events: api.MailCalendarEventPreview[];
  busy: api.MailCalendarBusySlot[];
  mailError: boolean;
  calendarError: boolean;
};

function providerName(account: api.MailCalendarAccount) {
  const name = account.provider === "google" ? "Google" : account.provider === "microsoft" ? "Microsoft" : "Apple iCloud";
  return account.identity_masked ? `${name} · ${account.identity_masked}` : name;
}

async function readToday(agentId: string): Promise<{ day: string; accounts: DailyAccount[]; refreshedAt: string }> {
  const startDate = new Date();
  const day = dateKey(startDate);
  const end = tomorrow(startDate);
  const fromInstant = new Date(`${day}T00:00:00`);
  const toInstant = new Date(`${end}T00:00:00`);
  const from = fromInstant.toISOString();
  const to = toInstant.toISOString();
  const inventory = await api.listMailCalendarAccounts(agentId);
  const accounts = inventory.accounts.filter((account) => account.status === "connected" && !account.revoked_at);
  const results: DailyAccount[] = [];
  // A Today view can span several linked accounts. Keep provider fan-out
  // bounded while retaining the combined agenda and inbox.
  for (let offset = 0; offset < accounts.length; offset += 2) {
    const batch = accounts.slice(offset, offset + 2);
    const batchResults = await Promise.all(batch.map(async (account): Promise<DailyAccount> => {
      const [mail, calendar, freeBusy] = await Promise.all([
        account.capabilities.includes("mail_read") && account.credential_available
          ? api.previewMailCalendarMail(agentId, account.id, 8).then((value) => ({ value: value.messages, failed: false })).catch(() => ({ value: [] as api.MailCalendarMailPreview[], failed: true }))
          : Promise.resolve({ value: [] as api.MailCalendarMailPreview[], failed: false }),
        account.capabilities.includes("calendar_read") && account.credential_available
          ? api.previewMailCalendarEvents(agentId, account.id, from, to, 50).then((value) => ({ value: value.events.map((event) => ({ ...event, account_id: account.id, account_name: providerName(account) })), failed: false })).catch(() => ({ value: [] as api.MailCalendarEventPreview[], failed: true }))
          : Promise.resolve({ value: [] as api.MailCalendarEventPreview[], failed: false }),
        account.capabilities.includes("calendar_free_busy") && !account.capabilities.includes("calendar_read") && account.credential_available
          ? api.previewMailCalendarFreeBusy(agentId, account.id, from, to).then((value) => ({ value: value.busy, failed: false })).catch(() => ({ value: [] as api.MailCalendarBusySlot[], failed: true }))
          : Promise.resolve({ value: [] as api.MailCalendarBusySlot[], failed: false }),
      ]);
      const needsCredential = !account.credential_available;
      return {
        account,
        messages: mail.value,
        events: calendar.value,
        busy: freeBusy.value,
        mailError: mail.failed || (needsCredential && account.capabilities.includes("mail_read")),
        calendarError: calendar.failed || freeBusy.failed || (needsCredential && (account.capabilities.includes("calendar_read") || account.capabilities.includes("calendar_free_busy"))),
      };
    }));
    results.push(...batchResults);
  }
  return { day, accounts: results, refreshedAt: new Date().toLocaleTimeString([], { hour: "numeric", minute: "2-digit" }) };
}

export default function DailyMailCalendarViewer(props: ViewerProps) {
  const loader = createLoader(
    () => [props.subject.kind === "daily_mail_calendar" ? props.subject.agentId : "", props.reloadKey] as const,
    ([agentId]) => readToday(agentId),
  );
  const [selectedEvent, setSelectedEvent] = createSignal<api.MailCalendarEventPreview | null>(null);
  const [calendarAccountFilter, setCalendarAccountFilter] = createSignal("all");
  const [eventAction, setEventAction] = createSignal("");
  const [eventActionError, setEventActionError] = createSignal<string | null>(null);
  let lastRefresh = Date.now();
  const refreshIfStale = () => {
    if (document.visibilityState !== "visible" || loader.loading() || Date.now() - lastRefresh < 60_000) return;
    lastRefresh = Date.now();
    setSelectedEvent(null);
    loader.reload();
  };
  const manualRefresh = () => { lastRefresh = Date.now(); setSelectedEvent(null); loader.reload(); };
  onMount(() => {
    const timer = window.setInterval(() => {
      if (Date.now() - lastRefresh >= 5 * 60_000) refreshIfStale();
    }, 30_000);
    document.addEventListener("visibilitychange", refreshIfStale);
    window.addEventListener("focus", refreshIfStale);
    onCleanup(() => {
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", refreshIfStale);
      window.removeEventListener("focus", refreshIfStale);
    });
  });

  return <div class="daily-mail-calendar-view">
    <LoadState loader={loader}>{(data) => {
      const events = () => data.accounts.flatMap((account) => account.events);
      const visibleEvents = () => events().filter((event) => calendarAccountFilter() === "all" || event.account_id === calendarAccountFilter());
      const calendarAccounts = () => data.accounts.filter((account) => account.account.capabilities.includes("calendar_read") && account.events.length > 0);
      const hasMailCapability = () => data.accounts.some((account) => account.account.capabilities.includes("mail_read"));
      const hasCalendarCapability = () => data.accounts.some((account) => account.account.capabilities.includes("calendar_read") || account.account.capabilities.includes("calendar_free_busy"));
      const mailFailedAccounts = () => data.accounts.filter((account) => account.mailError);
      const calendarFailedAccounts = () => data.accounts.filter((account) => account.calendarError);
      const openSettings = () => { setPendingSettingsPage("mail-calendar"); setSettingsOpen(true); };
      const prepareEventAction = async (kind: "update" | "cancel") => {
        const event = selectedEvent();
        const agentId = props.subject.kind === "daily_mail_calendar" ? props.subject.agentId : "";
        if (!event || !agentId || !event.account_id || !event.version || eventAction()) return;
        setEventAction(kind);
        setEventActionError(null);
        try {
          const action: api.MailCalendarDraftAction = kind === "update"
            ? { kind: "update_event", event_id: event.provider_id, source_version: event.version, draft: { title: event.title, description: event.description ?? "", location: event.location, starts_at: event.starts_at!, ends_at: event.ends_at!, time_zone: Intl.DateTimeFormat().resolvedOptions().timeZone, all_day: false, attendee_addresses: [], recurrence: null, occurrence_id: null } }
            : { kind: "cancel_event", event_id: event.provider_id, source_version: event.version, occurrence_id: null, whole_series: false };
          await api.saveMailCalendarCandidate(agentId, { account_id: event.account_id, source_refs: [{ item_id: event.provider_id, version: event.version, label: event.title }], action });
          openSettings();
        } catch (cause) {
          setEventActionError(cause instanceof Error ? cause.message : "Could not prepare a local draft.");
        } finally {
          setEventAction("");
        }
      };
      return <main class="daily-mail-calendar-body">
        <header class="daily-mail-calendar-heading"><div><h2>{new Date(`${data.day}T12:00:00`).toLocaleDateString([], { weekday: "long", month: "long", day: "numeric" })}</h2><p>From connected accounts · updated {data.refreshedAt}</p></div><button type="button" class="artifact-canvas-btn" onClick={manualRefresh}>Refresh</button></header>
        <Show when={data.accounts.length === 0}>
          <section class="daily-mail-calendar-empty"><h3>No connected accounts</h3><p>Connect mail or a calendar to see today’s agenda and recent messages here.</p><button type="button" class="artifact-canvas-btn" onClick={openSettings}>Open email and calendar settings</button></section>
        </Show>
        <Show when={data.accounts.length > 0}>
          <section class="daily-mail-calendar-section"><div class="daily-mail-calendar-calendar-heading"><h3>Today’s calendar</h3><div class="daily-mail-calendar-calendar-tools"><label>Show calendars<select aria-label="Show calendars" value={calendarAccountFilter()} onChange={(event) => { setCalendarAccountFilter(event.currentTarget.value); setSelectedEvent(null); }}><option value="all">All calendars ({events().length})</option><For each={calendarAccounts()}>{(account) => <option value={account.account.id}>{providerName(account.account)} ({account.events.length})</option>}</For></select></label><span>Auto-refreshes every 5 minutes while open</span></div></div>
            <Show when={!hasCalendarCapability()}><p class="settings-hint">No connected account has calendar access.</p></Show>
            <For each={calendarFailedAccounts()}>{(account) => <p class="daily-mail-calendar-warning" role="status">Calendar unavailable for {providerName(account.account)}. Check this account in Settings.</p>}</For>
            <Show when={selectedEvent()}>{(event) => <aside class="daily-mail-calendar-event-detail" aria-label="Selected event details"><div class="daily-mail-calendar-event-detail-heading"><div><span>Event details</span><h4>{event().title}</h4></div><button type="button" class="settings-button" onClick={() => setSelectedEvent(null)}>Close</button></div><dl><div><dt>When</dt><dd>{event().all_day ? "All day" : event().starts_at && event().ends_at ? `${new Date(event().starts_at!).toLocaleString()} – ${new Date(event().ends_at!).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}` : "Time unavailable"}</dd></div><div><dt>Calendar</dt><dd>{event().account_name ?? "Connected calendar"}</dd></div><Show when={event().location}><div><dt>Location</dt><dd>{event().location}</dd></div></Show><Show when={event().description}><div><dt>Description</dt><dd>{event().description}</dd></div></Show></dl><p>This view is read-only. Prepare a change as a local draft, then review its exact effect before anything is sent to the provider.</p><div class="settings-actions"><Show when={event().account_id && event().version && event().provider_id && event().starts_at && event().ends_at && !event().private && !event().all_day && !event().recurring && event().attendee_count === 0 && event().account_name?.startsWith("Google")}><button type="button" class="settings-button" disabled={!!eventAction()} onClick={() => void prepareEventAction("update")}>{eventAction() === "update" ? "Preparing draft…" : "Draft an update"}</button></Show><Show when={event().can_cancel && event().account_id && event().version}><button type="button" class="settings-button danger" disabled={!!eventAction()} onClick={() => void prepareEventAction("cancel")}>{eventAction() === "cancel" ? "Preparing review…" : "Review cancellation"}</button></Show><button type="button" class="settings-button" onClick={openSettings}>Open calendar workspace</button></div><Show when={eventActionError()}><p role="alert" class="daily-mail-calendar-warning">{eventActionError()}</p></Show></aside>}</Show>
            <Show when={visibleEvents().length > 0}><MailCalendarAgenda events={visibleEvents()} from={data.day} to={data.day} conflicts={new Set()} initialView="day" onSelect={setSelectedEvent} /></Show>
            <Show when={hasCalendarCapability() && events().length === 0 && calendarFailedAccounts().length === 0}><p class="settings-hint">No events today.</p></Show>
            <For each={data.accounts.filter((account) => account.busy.length > 0)}>{(account) => <div class="daily-mail-calendar-busy"><strong>Busy · {providerName(account.account)}</strong><For each={account.busy}>{(slot) => <span>{new Date(slot.starts_at).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}–{new Date(slot.ends_at).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}</span>}</For></div>}</For>
          </section>
          <section class="daily-mail-calendar-section"><h3>Recent email</h3>
            <Show when={!hasMailCapability()}><p class="settings-hint">No connected account has email access.</p></Show>
            <For each={mailFailedAccounts()}>{(account) => <p class="daily-mail-calendar-warning" role="status">Inbox unavailable for {providerName(account.account)}. Check this account in Settings.</p>}</For>
            <For each={data.accounts.filter((account) => account.messages.length > 0)}>{(account) => <div class="daily-mail-calendar-mail-account"><h4>{providerName(account.account)}</h4><For each={account.messages}>{(message) => <article class="daily-mail-calendar-message"><div><strong>{message.subject || "(No subject)"}</strong><small>{message.from || "Sender unavailable"}{message.received_at ? ` · ${new Date(message.received_at).toLocaleString()}` : ""}</small><Show when={message.preview}><p>{message.preview}</p></Show></div><Show when={message.thread_id} fallback={<button type="button" class="settings-button" onClick={openSettings}>Open email</button>}>{(threadId) => <button type="button" class="settings-button" onClick={() => openMailCalendarCitation({ accountId: account.account.id, threadId: threadId(), messageId: message.provider_id })}>Open conversation</button>}</Show></article>}</For></div>}</For>
            <Show when={hasMailCapability() && data.accounts.every((account) => account.messages.length === 0) && mailFailedAccounts().length === 0}><p class="settings-hint">No recent messages.</p></Show>
          </section>
        </Show>
        <footer class="daily-mail-calendar-privacy">Read-only preview for this Agent and your local owner session. Refresh runs every five minutes while visible and when you return after a minute away. Provider content is not added to the conversation by opening this view.</footer>
      </main>;
    }}</LoadState>
  </div>;
}
