import { For, Show } from "solid-js";
import * as api from "../../api";
import { openMailCalendarCitation, setPendingSettingsPage, setSettingsOpen } from "../../store";
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
  const results = await Promise.all(accounts.map(async (account): Promise<DailyAccount> => {
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
  return { day, accounts: results, refreshedAt: new Date().toLocaleTimeString([], { hour: "numeric", minute: "2-digit" }) };
}

export default function DailyMailCalendarViewer(props: ViewerProps) {
  const loader = createLoader(
    () => [props.subject.kind === "daily_mail_calendar" ? props.subject.agentId : "", props.reloadKey] as const,
    ([agentId]) => readToday(agentId),
  );

  return <div class="daily-mail-calendar-view">
    <LoadState loader={loader}>{(data) => {
      const events = () => data.accounts.flatMap((account) => account.events);
      const hasMailCapability = () => data.accounts.some((account) => account.account.capabilities.includes("mail_read"));
      const hasCalendarCapability = () => data.accounts.some((account) => account.account.capabilities.includes("calendar_read") || account.account.capabilities.includes("calendar_free_busy"));
      const mailFailedAccounts = () => data.accounts.filter((account) => account.mailError);
      const calendarFailedAccounts = () => data.accounts.filter((account) => account.calendarError);
      const openSettings = () => { setPendingSettingsPage("mail-calendar"); setSettingsOpen(true); };
      return <main class="daily-mail-calendar-body">
        <header class="daily-mail-calendar-heading"><div><h2>{new Date(`${data.day}T12:00:00`).toLocaleDateString([], { weekday: "long", month: "long", day: "numeric" })}</h2><p>From connected accounts · updated {data.refreshedAt}</p></div><button type="button" class="artifact-canvas-btn" onClick={loader.reload}>Refresh</button></header>
        <Show when={data.accounts.length === 0}>
          <section class="daily-mail-calendar-empty"><h3>No connected accounts</h3><p>Connect mail or a calendar to see today’s agenda and recent messages here.</p><button type="button" class="artifact-canvas-btn" onClick={openSettings}>Open email and calendar settings</button></section>
        </Show>
        <Show when={data.accounts.length > 0}>
          <section class="daily-mail-calendar-section"><h3>Today’s calendar</h3>
            <Show when={!hasCalendarCapability()}><p class="settings-hint">No connected account has calendar access.</p></Show>
            <For each={calendarFailedAccounts()}>{(account) => <p class="daily-mail-calendar-warning" role="status">Calendar unavailable for {providerName(account.account)}. Check this account in Settings.</p>}</For>
            <Show when={events().length > 0}><div class="mail-calendar-agenda-list"><For each={events()}>{(event) => <article class="mail-calendar-event"><div class="mail-calendar-event-time">{event.all_day ? "All day" : event.starts_at && event.ends_at ? `${new Date(event.starts_at).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}–${new Date(event.ends_at).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}` : "Time unavailable"}</div><div class="mail-calendar-event-content"><strong>{event.title}</strong><Show when={event.account_name}><small>{event.account_name}</small></Show><Show when={event.location}><span>{event.location}</span></Show></div></article>}</For></div></Show>
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
        <footer class="daily-mail-calendar-privacy">Read-only preview for this Agent and your local owner session. Provider content is not added to the conversation by opening this view.</footer>
      </main>;
    }}</LoadState>
  </div>;
}
