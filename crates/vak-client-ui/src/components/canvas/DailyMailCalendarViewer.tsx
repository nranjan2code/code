import { createEffect, For, Show, createSignal, onCleanup, onMount } from "solid-js";
import * as api from "../../api";
import { setSyntheticMailCalendarEnabled, syntheticMailCalendarEnabled } from "../../mailCalendarDemo";
import { openArtifactCanvas, pendingMailCalendarCitation, setPendingMailCalendarCitation, setPendingSettingsPage, setSettingsOpen } from "../../store";
import { loadConversationCitation } from "../../mailCalendarThreadNavigation.mjs";
import { MailCalendarAgenda } from "../MailCalendarAgenda";
import { SyntheticMailCalendarDemoButton } from "../SyntheticMailCalendarDemoControl";
import MailCalendarDraftWorkspace from "./MailCalendarDraftWorkspace";
import MailCalendarRoutineWorkspace from "./MailCalendarRoutineWorkspace";
import { createLoader } from "./createLoader";
import LoadState from "./LoadState";
import type { ViewerProps } from "./types";

const dateKey = (date: Date) => `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
const prettyDay = (day: string) => new Date(`${day}T12:00:00`).toLocaleDateString([], { weekday: "long", month: "long", day: "numeric" });
const tomorrow = (date: Date) => { const next = new Date(date); next.setDate(next.getDate() + 1); return dateKey(next); };
const inclusiveDays = (from: string, through: string) => Math.round((Date.parse(`${through}T12:00:00`) - Date.parse(`${from}T12:00:00`)) / 86_400_000) + 1;
const addDays = (day: string, amount: number) => { const date = new Date(`${day}T12:00:00`); date.setDate(date.getDate() + amount); return dateKey(date); };

type DailyAccount = {
  account: api.MailCalendarAccount;
  messages: api.MailCalendarMailPreview[];
  events: api.MailCalendarEventPreview[];
  busy: api.MailCalendarBusySlot[];
  mailError: boolean;
  calendarError: boolean;
};

type SelectedConversation = {
  accountId: string;
  threadId: string;
  messages: api.MailCalendarMailPreview[];
  nextCursor?: string | null;
};

function providerName(account: api.MailCalendarAccount) {
  const name = account.provider === "google" ? "Google" : account.provider === "microsoft" ? "Microsoft" : "Apple iCloud";
  return account.identity_masked ? `${name} · ${account.identity_masked}` : name;
}

async function readToday(agentId: string, day: string, through: string, mailAccountId: string, query: string, folderId: string): Promise<{ day: string; through: string; mailAccountId: string; mailQuery: string; mailFolderId: string; accounts: DailyAccount[]; refreshedAt: string }> {
  const fromInstant = new Date(`${day}T00:00:00`);
  const toInstant = new Date(`${tomorrow(new Date(`${through}T12:00:00`))}T00:00:00`);
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
        account.capabilities.includes("mail_read") && account.credential_available && (mailAccountId === "all" || mailAccountId === account.id)
          ? api.previewMailCalendarMail(agentId, account.id, 8, query, mailAccountId === account.id ? folderId || undefined : undefined).then((value) => ({ value: value.messages, failed: false })).catch(() => ({ value: [] as api.MailCalendarMailPreview[], failed: true }))
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
  return { day, through, mailAccountId, mailQuery: query, mailFolderId: folderId, accounts: results, refreshedAt: new Date().toLocaleTimeString([], { hour: "numeric", minute: "2-digit" }) };
}

export default function DailyMailCalendarViewer(props: ViewerProps) {
  const [rangeFrom, setRangeFrom] = createSignal(dateKey(new Date()));
  const [rangeThrough, setRangeThrough] = createSignal(dateKey(new Date()));
  const [rangeError, setRangeError] = createSignal("");
  const [mailAccountFilter, setMailAccountFilter] = createSignal("all");
  const [mailFolderFilter, setMailFolderFilter] = createSignal("");
  const [mailFolders, setMailFolders] = createSignal<api.MailCalendarFolder[]>([]);
  const [mailFoldersLoading, setMailFoldersLoading] = createSignal(false);
  const [mailFoldersError, setMailFoldersError] = createSignal("");
  const [mailSearchInput, setMailSearchInput] = createSignal("");
  const [mailSearchQuery, setMailSearchQuery] = createSignal("");
  let folderRequestGeneration = 0;
  const loader = createLoader(
    () => [props.subject.kind === "daily_mail_calendar" ? props.subject.agentId : "", props.reloadKey] as const,
    ([agentId]) => readToday(agentId, rangeFrom(), rangeThrough(), mailAccountFilter(), mailSearchQuery(), mailFolderFilter()),
  );
  const [selectedEvent, setSelectedEvent] = createSignal<api.MailCalendarEventPreview | null>(null);
  const [selectedConversation, setSelectedConversation] = createSignal<SelectedConversation | null>(null);
  const [conversationLoading, setConversationLoading] = createSignal(false);
  const [conversationError, setConversationError] = createSignal<string | null>(null);
  const [conversationRequest, setConversationRequest] = createSignal(0);
  const [calendarAccountFilter, setCalendarAccountFilter] = createSignal("all");
  const [calendarPage, setCalendarPage] = createSignal(0);
  const [mailPage, setMailPage] = createSignal(0);
  const [eventAction, setEventAction] = createSignal("");
  const [eventActionError, setEventActionError] = createSignal<string | null>(null);
  const [accountRefreshPending, setAccountRefreshPending] = createSignal(false);
  const [syntheticDemo, setSyntheticDemo] = createSignal(syntheticMailCalendarEnabled());
  const freshnessLabel = () => `${syntheticDemo() ? "Synthetic demo data · no provider connected" : "From connected accounts"} · updated ${loader.data()?.refreshedAt ?? ""}`;
  const applyDateRange = () => {
    const count = inclusiveDays(rangeFrom(), rangeThrough());
    if (!Number.isFinite(count) || count < 1 || count > 30) {
      setRangeError("Choose an end date on or after the start date, within a 30-day range.");
      return;
    }
    setRangeError("");
    setSelectedEvent(null);
    setCalendarPage(0);
    loader.reload();
  };
  const selectMailAccount = async (accountId: string) => {
    setMailAccountFilter(accountId);
    setMailFolderFilter("");
    setMailFolders([]);
    setMailFoldersError("");
    setMailSearchInput(""); setMailSearchQuery(""); setMailPage(0);
    closeConversation();
    const agentId = props.subject.kind === "daily_mail_calendar" ? props.subject.agentId : "";
    const generation = ++folderRequestGeneration;
    if (accountId === "all" || !agentId) { loader.reload(); return; }
    setMailFoldersLoading(true);
    try {
      const result = await api.listMailCalendarFolders(agentId, accountId);
      if (generation !== folderRequestGeneration || accountId !== mailAccountFilter()) return;
      setMailFolders(result.folders);
      const inbox = result.folders.find((folder) => folder.name.toLowerCase() === "inbox") ?? result.folders[0];
      setMailFolderFilter(inbox?.provider_id ?? "");
    } catch (cause) {
      if (generation === folderRequestGeneration) setMailFoldersError(cause instanceof Error ? cause.message : "Could not load this account’s folders.");
    } finally {
      if (generation === folderRequestGeneration) {
        setMailFoldersLoading(false);
        loader.reload();
      }
    }
  };
  const applyMailSearch = () => {
    setMailSearchQuery(mailSearchInput().trim().slice(0, 128));
    setMailPage(0);
    closeConversation();
    loader.reload();
  };
  let lastRefresh = Date.now();
  const refreshIfStale = () => {
    if (document.visibilityState !== "visible" || loader.loading() || Date.now() - lastRefresh < 60_000) return;
    lastRefresh = Date.now();
    setSelectedEvent(null);
    loader.reload();
  };
  const refreshAfterAccountChange = () => {
    setSyntheticDemo(syntheticMailCalendarEnabled());
    if (document.visibilityState !== "visible") return;
    if (loader.loading()) {
      setAccountRefreshPending(true);
      return;
    }
    lastRefresh = Date.now();
    setSelectedEvent(null);
    loader.reload();
  };
  createEffect(() => {
    if (!accountRefreshPending() || loader.loading()) return;
    setAccountRefreshPending(false);
    lastRefresh = Date.now();
    setSelectedEvent(null);
    loader.reload();
  });
  const manualRefresh = () => { lastRefresh = Date.now(); setSelectedEvent(null); loader.reload(); };
  const closeConversation = () => {
    setConversationRequest((value) => value + 1);
    setSelectedConversation(null);
    setConversationError(null);
    setConversationLoading(false);
  };
  const readConversation = async (accountId: string, threadId: string, cursor?: string, append = false, targetMessageId?: string) => {
    const request = conversationRequest() + 1;
    setConversationRequest(request);
    if (!append) setSelectedConversation({ accountId, threadId, messages: [] });
    setConversationLoading(true);
    setConversationError(null);
    try {
      const page = await api.previewMailCalendarThread(
        props.subject.kind === "daily_mail_calendar" ? props.subject.agentId : "",
        accountId,
        threadId,
        cursor,
      );
      if (conversationRequest() !== request) return;
      const located = targetMessageId
        ? await loadConversationCitation(page, targetMessageId, (nextCursor) => api.previewMailCalendarThread(
          props.subject.kind === "daily_mail_calendar" ? props.subject.agentId : "",
          accountId,
          threadId,
          nextCursor,
        ))
        : { messages: page.messages, nextCursor: page.next_cursor, found: true };
      if (conversationRequest() !== request) return;
      setSelectedConversation((current) => {
        if (!current || current.accountId !== accountId || current.threadId !== threadId) return current;
        const messages = append ? [...current.messages] : [];
        const seen = new Set(messages.map((message) => message.provider_id));
        for (const message of located.messages) {
          if (!seen.has(message.provider_id)) {
            seen.add(message.provider_id);
            messages.push(message);
          }
        }
        return { accountId, threadId, messages, nextCursor: located.nextCursor };
      });
      if (targetMessageId && !located.found) setConversationError(located.nextCursor
        ? "This cited message is beyond the automatic preview limit. Load more messages to continue."
        : "This cited message was not found in the available conversation pages.");
      if (targetMessageId && located.found) window.requestAnimationFrame(() => {
        const message = document.querySelector<HTMLElement>(`[data-mail-message-id="${CSS.escape(targetMessageId)}"]`);
        message?.scrollIntoView({ behavior: "smooth", block: "center" });
        message?.focus({ preventScroll: true });
      });
    } catch (cause) {
      if (conversationRequest() === request) {
        setConversationError(cause instanceof Error ? cause.message : "Could not load this conversation.");
      }
    } finally {
      if (conversationRequest() === request) setConversationLoading(false);
    }
  };
  createEffect(() => {
    const citation = pendingMailCalendarCitation();
    const data = loader.data();
    if (!citation || !data) return;
    const account = data.accounts.find((item) => item.account.id === citation.accountId)?.account;
    setPendingMailCalendarCitation(null);
    if (!account || account.provider === "apple_icloud" || !account.capabilities.includes("mail_read")) {
      setConversationError("This conversation citation is not available in this Agent’s connected mail accounts.");
      return;
    }
    void readConversation(account.id, citation.threadId, undefined, false, citation.messageId);
  });
  onMount(() => {
    const timer = window.setInterval(() => {
      if (Date.now() - lastRefresh >= 5 * 60_000) refreshIfStale();
    }, 30_000);
    document.addEventListener("visibilitychange", refreshIfStale);
    window.addEventListener("focus", refreshIfStale);
    window.addEventListener("online", refreshIfStale);
    window.addEventListener("vak:mail-calendar-changed", refreshAfterAccountChange);
    onCleanup(() => {
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", refreshIfStale);
      window.removeEventListener("focus", refreshIfStale);
      window.removeEventListener("online", refreshIfStale);
      window.removeEventListener("vak:mail-calendar-changed", refreshAfterAccountChange);
    });
  });

  return <div class="daily-mail-calendar-view">
    <LoadState loader={loader}>{(data) => {
      const events = () => data.accounts.flatMap((account) => account.events);
      const visibleEvents = () => events().filter((event) => calendarAccountFilter() === "all" || event.account_id === calendarAccountFilter());
      const calendarPageCount = () => Math.max(1, Math.ceil(inclusiveDays(data.day, data.through) / 7));
      const calendarPageFrom = () => addDays(data.day, calendarPage() * 7);
      const calendarPageThrough = () => calendarPage() + 1 >= calendarPageCount()
        ? data.through
        : addDays(calendarPageFrom(), 6);
      const calendarPageEvents = () => visibleEvents().filter((event) => {
        const day = event.starts_on ?? (event.starts_at ? dateKey(new Date(event.starts_at)) : "");
        return day >= calendarPageFrom() && day <= calendarPageThrough();
      });
      const calendarAccounts = () => data.accounts.filter((account) => account.account.capabilities.includes("calendar_read") && account.events.length > 0);
      const hasMailCapability = () => data.accounts.some((account) => account.account.capabilities.includes("mail_read"));
      const hasCalendarCapability = () => data.accounts.some((account) => account.account.capabilities.includes("calendar_read") || account.account.capabilities.includes("calendar_free_busy"));
      const mailFailedAccounts = () => data.accounts.filter((account) => account.mailError);
      const mailRows = () => data.accounts.flatMap((account) => account.messages.map((message) => ({ account, message })));
      const mailPageSize = 12;
      const mailPageCount = () => Math.max(1, Math.ceil(mailRows().length / mailPageSize));
      const visibleMailRows = () => mailRows().slice(mailPage() * mailPageSize, (mailPage() + 1) * mailPageSize);
      const visibleMailAccounts = () => data.accounts.flatMap((account) => {
        const messages = visibleMailRows().filter((row) => row.account.account.id === account.account.id).map((row) => row.message);
        return messages.length ? [{ ...account, messages }] : [];
      });
      const calendarFailedAccounts = () => data.accounts.filter((account) => account.calendarError);
      const openSettings = () => { setPendingSettingsPage("mail-calendar"); setSettingsOpen(true); };
      const openDraftWorkspace = (candidateId?: string) => window.dispatchEvent(new CustomEvent("vak:mail-calendar-open-candidate", {
        detail: { agentId: props.subject.kind === "daily_mail_calendar" ? props.subject.agentId : "", candidateId },
      }));
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
          const saved = await api.saveMailCalendarCandidate(agentId, { account_id: event.account_id, source_refs: [{ item_id: event.provider_id, version: event.version, label: event.title }], action });
          openDraftWorkspace(saved.candidate.id);
        } catch (cause) {
          setEventActionError(cause instanceof Error ? cause.message : "Could not prepare a local draft.");
        } finally {
          setEventAction("");
        }
      };
      return <main class="daily-mail-calendar-body">
        <header class="daily-mail-calendar-heading"><div><h2>{prettyDay(data.day)}</h2><p>{freshnessLabel()}</p></div><div class="settings-actions"><SyntheticMailCalendarDemoButton checked={syntheticDemo()} onChange={setSyntheticMailCalendarEnabled} /><button type="button" class="artifact-canvas-btn" onClick={manualRefresh}>Refresh</button></div></header>
        <Show when={data.accounts.length === 0}>
          <section class="daily-mail-calendar-empty"><h3>No connected accounts</h3><p>Connect mail or a calendar to see today’s agenda and recent messages here.</p><button type="button" class="artifact-canvas-btn" onClick={openSettings}>Open email and calendar settings</button></section>
        </Show>
        <Show when={data.accounts.length > 0}>
            <section class="daily-mail-calendar-section"><div class="daily-mail-calendar-calendar-heading"><div><h3>{data.day === data.through ? data.day === dateKey(new Date()) ? "Today’s calendar" : prettyDay(data.day) : `${prettyDay(data.day)} – ${prettyDay(data.through)} · ${inclusiveDays(data.day, data.through)} days`}</h3><p>Choose up to 30 days · times use your device time zone</p></div><div class="daily-mail-calendar-calendar-tools"><label>From<input aria-label="Calendar start date" type="date" value={rangeFrom()} onInput={(event) => setRangeFrom(event.currentTarget.value)} /></label><label>Through<input aria-label="Calendar end date" type="date" value={rangeThrough()} onInput={(event) => setRangeThrough(event.currentTarget.value)} /></label><button type="button" class="settings-button" onClick={() => { setRangeThrough(addDays(rangeFrom(), 6)); setRangeError(""); }}>Use 7 days</button><button type="button" class="settings-button" onClick={applyDateRange}>Refresh dates</button><label>Show calendars<select aria-label="Show calendars" value={calendarAccountFilter()} onChange={(event) => { setCalendarAccountFilter(event.currentTarget.value); setSelectedEvent(null); }}><option value="all">All calendars ({events().length})</option><For each={calendarAccounts()}>{(account) => <option value={account.account.id}>{providerName(account.account)} ({account.events.length})</option>}</For></select></label><span>Auto-refreshes every 5 minutes while open</span></div></div>
            <Show when={rangeError()}><p class="daily-mail-calendar-warning" role="alert">{rangeError()}</p></Show>
            <Show when={!hasCalendarCapability()}><p class="settings-hint">No connected account has calendar access.</p></Show>
            <For each={calendarFailedAccounts()}>{(account) => <p class="daily-mail-calendar-warning" role="status">Calendar unavailable for {providerName(account.account)}. Check this account in Settings.</p>}</For>
            <Show when={data.accounts.some((account) => account.events.length >= 50)}><p class="settings-hint" role="status">A calendar reached the 50-event preview limit. Narrow the date range if more events may be available.</p></Show>
            <Show when={selectedEvent()}>{(event) => <aside class="daily-mail-calendar-event-detail" aria-label="Selected event details"><div class="daily-mail-calendar-event-detail-heading"><div><span>Event details</span><h4>{event().title}</h4></div><button type="button" class="settings-button" onClick={() => setSelectedEvent(null)}>Close</button></div><dl><div><dt>When</dt><dd>{event().all_day ? "All day" : event().starts_at && event().ends_at ? `${new Date(event().starts_at!).toLocaleString()} – ${new Date(event().ends_at!).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}` : "Time unavailable"}</dd></div><div><dt>Calendar</dt><dd>{event().account_name ?? "Connected calendar"}</dd></div><Show when={event().location}><div><dt>Location</dt><dd>{event().location}</dd></div></Show><Show when={event().description}><div><dt>Description</dt><dd>{event().description}</dd></div></Show></dl><p>This view is read-only. Prepare a change as a local draft, then review its exact effect before anything is sent to the provider.</p><div class="settings-actions"><Show when={event().account_id && event().version && event().provider_id && event().starts_at && event().ends_at && !event().private && !event().all_day && !event().recurring && event().attendee_count === 0 && event().account_name?.startsWith("Google")}><button type="button" class="settings-button" disabled={!!eventAction()} onClick={() => void prepareEventAction("update")}>{eventAction() === "update" ? "Preparing draft…" : "Draft an update"}</button></Show><Show when={event().can_cancel && event().account_id && event().version}><button type="button" class="settings-button danger" disabled={!!eventAction()} onClick={() => void prepareEventAction("cancel")}>{eventAction() === "cancel" ? "Preparing review…" : "Review cancellation"}</button></Show><button type="button" class="settings-button" onClick={() => openDraftWorkspace()}>Open drafts and review</button></div><Show when={eventActionError()}><p role="alert" class="daily-mail-calendar-warning">{eventActionError()}</p></Show></aside>}</Show>
            <Show when={calendarPageCount() > 1}><nav class="daily-mail-calendar-pagination" aria-label="Calendar week pages"><button type="button" class="settings-button" disabled={calendarPage() === 0} onClick={() => { setCalendarPage((page) => Math.max(0, page - 1)); setSelectedEvent(null); }}>Previous week</button><span aria-live="polite">Week {calendarPage() + 1} of {calendarPageCount()} · {prettyDay(calendarPageFrom())} – {prettyDay(calendarPageThrough())}</span><button type="button" class="settings-button" disabled={calendarPage() + 1 >= calendarPageCount()} onClick={() => { setCalendarPage((page) => Math.min(calendarPageCount() - 1, page + 1)); setSelectedEvent(null); }}>Next week</button></nav></Show>
            <Show when={calendarPageEvents().length > 0}><For each={[calendarAccountFilter()]}>{() => <MailCalendarAgenda events={calendarPageEvents()} from={calendarPageFrom()} to={calendarPageThrough()} conflicts={new Set()} initialView={calendarPageFrom() === calendarPageThrough() ? "day" : "week"} onSelect={setSelectedEvent} />}</For></Show>
            <Show when={hasCalendarCapability() && calendarPageEvents().length === 0 && calendarFailedAccounts().length === 0}><p class="settings-hint">No events for this date range.</p></Show>
            <For each={data.accounts.filter((account) => account.busy.length > 0)}>{(account) => <div class="daily-mail-calendar-busy"><strong>Busy · {providerName(account.account)}</strong><For each={account.busy.filter((slot) => { const day = dateKey(new Date(slot.starts_at)); return day >= calendarPageFrom() && day <= calendarPageThrough(); })}>{(slot) => <span>{new Date(slot.starts_at).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}–{new Date(slot.ends_at).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}</span>}</For></div>}</For>
          </section>
          <section class="daily-mail-calendar-section"><h3>Recent email</h3>
            <div class="daily-mail-calendar-mail-filters"><label>Mailbox<select aria-label="Mail account" value={mailAccountFilter()} onChange={(event) => void selectMailAccount(event.currentTarget.value)}><option value="all">All inboxes</option><For each={data.accounts.filter((account) => account.account.capabilities.includes("mail_read"))}>{(account) => <option value={account.account.id}>{providerName(account.account)}</option>}</For></select></label><Show when={mailAccountFilter() !== "all"}><label>Folder or label<select aria-label="Mail folder or label" value={mailFolderFilter()} disabled={mailFoldersLoading() || mailFolders().length === 0} onChange={(event) => setMailFolderFilter(event.currentTarget.value)}><For each={mailFolders()}>{(folder) => <option value={folder.provider_id}>{folder.name}</option>}</For></select></label></Show><label>Search {mailAccountFilter() === "all" ? "in inboxes" : "this folder"}<input aria-label="Search mail" type="search" maxlength="128" value={mailSearchInput()} onInput={(event) => setMailSearchInput(event.currentTarget.value)} placeholder="Sender, subject or message" /></label><button type="button" class="settings-button" disabled={mailFoldersLoading()} onClick={applyMailSearch}>{mailFoldersLoading() ? "Loading folders…" : "Search mail"}</button></div>
            <Show when={mailFoldersError()}><p class="daily-mail-calendar-warning" role="alert">{mailFoldersError()}</p></Show>
            <p class="settings-hint">{data.mailAccountId === "all" ? "Showing recent inbox messages across connected accounts." : `Showing ${data.accounts.find((account) => account.account.id === data.mailAccountId)?.account.identity_masked ?? "the selected account"}${data.mailFolderId ? ` · ${mailFolders().find((folder) => folder.provider_id === data.mailFolderId)?.name ?? "selected folder"}` : " · Inbox"}${data.mailQuery ? ` · Search: ${data.mailQuery}` : ""}`}</p>
            <Show when={!hasMailCapability()}><p class="settings-hint">No connected account has email access.</p></Show>
            <For each={mailFailedAccounts()}>{(account) => <p class="daily-mail-calendar-warning" role="status">Inbox unavailable for {providerName(account.account)}. Check this account in Settings.</p>}</For>
            <For each={visibleMailAccounts()}>{(account) => <div class="daily-mail-calendar-mail-account"><h4>{providerName(account.account)}</h4><For each={account.messages}>{(message) => <article class="daily-mail-calendar-message"><div><strong>{message.subject || "(No subject)"}</strong><small>{message.from || "Sender unavailable"}{message.received_at ? ` · ${new Date(message.received_at).toLocaleString()}` : ""}</small><Show when={message.preview}><p>{message.preview}</p></Show></div><Show when={message.thread_id} fallback={<button type="button" class="settings-button" onClick={openSettings}>Open email</button>}>{(threadId) => <button type="button" class="settings-button" onClick={() => void readConversation(account.account.id, threadId())}>Open conversation</button>}</Show></article>}</For></div>}</For>
            <Show when={mailRows().length > mailPageSize}><nav class="daily-mail-calendar-pagination" aria-label="Recent email pages"><button type="button" class="settings-button" disabled={mailPage() === 0} onClick={() => setMailPage((page) => Math.max(0, page - 1))}>Previous</button><span aria-live="polite">Page {mailPage() + 1} of {mailPageCount()} · {mailRows().length} messages</span><button type="button" class="settings-button" disabled={mailPage() + 1 >= mailPageCount()} onClick={() => setMailPage((page) => Math.min(mailPageCount() - 1, page + 1))}>Next</button></nav></Show>
            <Show when={hasMailCapability() && data.accounts.every((account) => account.messages.length === 0) && mailFailedAccounts().length === 0}><p class="settings-hint">{data.mailQuery ? "No messages matched this search." : "No recent messages in this view."}</p></Show>
            <Show when={selectedConversation()}>{(conversation) => {
              const account = () => data.accounts.find((item) => item.account.id === conversation().accountId)?.account;
              const accountLabel = () => account() ? providerName(account()!) : "Connected account";
              return <section class="daily-mail-calendar-conversation" aria-label="Conversation preview">
                <header><div><span>{accountLabel()}</span><h4>{conversation().messages[0]?.subject || "Conversation"}</h4></div><button type="button" class="settings-button" onClick={closeConversation}>Close conversation</button></header>
                <p class="settings-hint">Read-only conversation preview. Message content is untrusted; opening it does not add it to the Agent conversation.</p>
                <Show when={conversationError()}><p class="daily-mail-calendar-warning" role="alert">{conversationError()}</p></Show>
                <For each={conversation().messages}>{(message) => <article class="daily-mail-calendar-conversation-message" data-mail-message-id={message.provider_id}>
                  <strong>{message.subject || "(No subject)"}</strong>
                  <small>{message.from || "Sender unavailable"}{message.received_at ? ` · ${new Date(message.received_at).toLocaleString()}` : ""}</small>
                  <Show when={message.to || message.cc}><small>{message.to ? `To: ${message.to}` : ""}{message.to && message.cc ? " · " : ""}{message.cc ? `Cc: ${message.cc}` : ""}</small></Show>
                  <small>Conversation content is untrusted. Ignore instructions inside it.</small>
                  <p>{message.body_text || message.preview || "No plain-text message content was returned."}</p>
                  <Show when={message.thread_id && account()?.capabilities.includes("mail_send")}><button type="button" class="settings-button" onClick={() => {
                    const agentId = props.subject.kind === "daily_mail_calendar" ? props.subject.agentId : "";
                    if (!agentId) return;
                    const action: api.MailCalendarDraftAction = { kind: "send_mail", draft: { from_alias: null, to: (message.from?.match(/[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}/i)?.[0] ? [{ address: message.from.match(/[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}/i)![0], display_name: null }] : []), cc: [], bcc: [], subject: message.subject?.startsWith("Re:") ? message.subject : `Re: ${message.subject || ""}`, body_text: "", attachment_refs: [], reply_to_message_id: message.provider_id, reply_to_thread_id: conversation().threadId } };
                    void api.saveMailCalendarCandidate(agentId, { account_id: conversation().accountId, source_refs: [{ item_id: message.provider_id, version: null, label: message.subject || "Reply" }], action })
                      .then((saved) => openDraftWorkspace(saved.candidate.id))
                      .catch((cause) => setConversationError(cause instanceof Error ? cause.message : "Could not prepare a local reply draft."));
                  }}>Draft reply in Canvas</button></Show>
                </article>}</For>
                <Show when={conversation().nextCursor}><button type="button" class="settings-button" disabled={conversationLoading()} onClick={() => void readConversation(conversation().accountId, conversation().threadId, conversation().nextCursor ?? undefined, true)}>{conversationLoading() ? "Loading more…" : "Load more messages"}</button></Show>
                <Show when={conversationLoading() && conversation().messages.length === 0}><p role="status">Loading conversation…</p></Show>
              </section>;
            }}</Show>
          </section>
        </Show>
        <Show when={props.subject.kind === "daily_mail_calendar"}><MailCalendarRoutineWorkspace agentId={props.subject.kind === "daily_mail_calendar" ? props.subject.agentId : ""} /></Show>
        <MailCalendarDraftWorkspace agentId={props.subject.kind === "daily_mail_calendar" ? props.subject.agentId : ""} />
        <footer class="daily-mail-calendar-privacy">Read-only preview for this Agent and your local owner session. Refresh runs every five minutes while visible and when you return after a minute away. Provider content is not added to the conversation by opening this view.</footer>
      </main>;
    }}</LoadState>
  </div>;
}
