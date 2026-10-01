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
  nextMailCursor: string | null;
  events: api.MailCalendarEventPreview[];
  nextCalendarCursor: string | null;
  busy: api.MailCalendarBusySlot[];
  mailError: boolean;
  calendarError: boolean;
};

type MailPageChunk = { messages: api.MailCalendarMailPreview[]; nextCursor: string | null };
type CalendarPageChunk = { events: api.MailCalendarEventPreview[]; nextCursor: string | null };

type SelectedConversation = {
  accountId: string;
  threadId: string | null;
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
          ? api.previewMailCalendarMail(agentId, account.id, 8, query, mailAccountId === account.id ? folderId || undefined : undefined).then((value) => ({ value, failed: false })).catch(() => ({ value: { messages: [] as api.MailCalendarMailPreview[], next_cursor: null }, failed: true }))
          : Promise.resolve({ value: { messages: [] as api.MailCalendarMailPreview[], next_cursor: null }, failed: false }),
        account.capabilities.includes("calendar_read") && account.credential_available
          ? api.previewMailCalendarEvents(agentId, account.id, from, to, 50).then((value) => ({ value: { events: value.events.map((event) => ({ ...event, account_id: account.id, account_name: providerName(account) })), cursor: value.next_cursor ?? null }, failed: false })).catch(() => ({ value: { events: [] as api.MailCalendarEventPreview[], cursor: null }, failed: true }))
          : Promise.resolve({ value: { events: [] as api.MailCalendarEventPreview[], cursor: null }, failed: false }),
        account.capabilities.includes("calendar_free_busy") && !account.capabilities.includes("calendar_read") && account.credential_available
          ? api.previewMailCalendarFreeBusy(agentId, account.id, from, to).then((value) => ({ value: value.busy, failed: false })).catch(() => ({ value: [] as api.MailCalendarBusySlot[], failed: true }))
          : Promise.resolve({ value: [] as api.MailCalendarBusySlot[], failed: false }),
      ]);
      const needsCredential = !account.credential_available;
      return {
        account,
        messages: mail.value.messages,
        nextMailCursor: mail.value.next_cursor ?? null,
        events: calendar.value.events,
        nextCalendarCursor: calendar.value.cursor,
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
  const [replyWorkspaceVisible, setReplyWorkspaceVisible] = createSignal(false);
  let lastOpenedMessage: { accountId: string; messageId: string } | null = null;
  const [conversationLoading, setConversationLoading] = createSignal(false);
  const [conversationError, setConversationError] = createSignal<string | null>(null);
  const [conversationRequest, setConversationRequest] = createSignal(0);
  const [attachmentPreview, setAttachmentPreview] = createSignal<{ accountId: string; messageId: string; attachmentId: string; filename: string; text: string } | null>(null);
  const [attachmentLoading, setAttachmentLoading] = createSignal<string | null>(null);
  const [attachmentError, setAttachmentError] = createSignal<string | null>(null);
  let attachmentRequest = 0;
  const [calendarAccountFilter, setCalendarAccountFilter] = createSignal("all");
  const [calendarPage, setCalendarPage] = createSignal(0);
  const [mailPage, setMailPage] = createSignal(0);
  const [mailAdditionalPages, setMailAdditionalPages] = createSignal<Record<string, MailPageChunk[]>>({});
  const [calendarAdditionalPages, setCalendarAdditionalPages] = createSignal<Record<string, CalendarPageChunk[]>>({});
  const [calendarLoadingMore, setCalendarLoadingMore] = createSignal(false);
  const [calendarLoadError, setCalendarLoadError] = createSignal("");
  let calendarPaginationGeneration = 0;
  const [mailLoadingMore, setMailLoadingMore] = createSignal(false);
  const [mailLoadError, setMailLoadError] = createSignal("");
  let mailPaginationGeneration = 0;
  let mailPaginationAgentId = props.subject.kind === "daily_mail_calendar" ? props.subject.agentId : "";
  createEffect(() => {
    const agentId = props.subject.kind === "daily_mail_calendar" ? props.subject.agentId : "";
    if (agentId === mailPaginationAgentId) return;
    mailPaginationAgentId = agentId;
    calendarPaginationGeneration += 1;
    setCalendarAdditionalPages({}); setCalendarLoadError(""); setCalendarLoadingMore(false);
    mailPaginationGeneration += 1;
    setMailAdditionalPages({}); setMailLoadError(""); setMailLoadingMore(false); setMailPage(0);
  });
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
    calendarPaginationGeneration += 1;
    setCalendarAdditionalPages({}); setCalendarLoadError("");
    setSelectedEvent(null);
    setCalendarPage(0);
    loader.reload();
  };
  const selectMailAccount = async (accountId: string) => {
    mailPaginationGeneration += 1;
    setMailAdditionalPages({}); setMailLoadError(""); setMailLoadingMore(false);
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
    mailPaginationGeneration += 1;
    setMailAdditionalPages({}); setMailLoadError(""); setMailLoadingMore(false);
    setMailSearchQuery(mailSearchInput().trim().slice(0, 128));
    setMailPage(0);
    closeConversation();
    loader.reload();
  };
  let lastRefresh = Date.now();
  const refreshIfStale = () => {
    if (document.visibilityState !== "visible" || loader.loading() || Date.now() - lastRefresh < 60_000) return;
    lastRefresh = Date.now();
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
    loader.reload();
  };
  createEffect(() => {
    if (!accountRefreshPending() || loader.loading()) return;
    setAccountRefreshPending(false);
    lastRefresh = Date.now();
    loader.reload();
  });
  const manualRefresh = () => { lastRefresh = Date.now(); loader.reload(); };
  const closeConversation = () => {
    setConversationRequest((value) => value + 1);
    setSelectedConversation(null);
    setReplyWorkspaceVisible(false);
    setConversationError(null);
    setConversationLoading(false);
    attachmentRequest += 1;
    setAttachmentLoading(null); setAttachmentPreview(null); setAttachmentError(null);
  };
  const closeConversationAndRestoreFocus = () => {
    const opener = lastOpenedMessage;
    closeConversation();
    if (opener) window.requestAnimationFrame(() => {
      document.querySelector<HTMLElement>(`[data-mail-open="${CSS.escape(`${opener.accountId}/${opener.messageId}`)}"]`)?.focus({ preventScroll: true });
    });
  };
  createEffect(() => {
    if (!selectedConversation()) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || event.defaultPrevented) return;
      event.preventDefault();
      closeConversationAndRestoreFocus();
    };
    document.addEventListener("keydown", onKeyDown);
    onCleanup(() => document.removeEventListener("keydown", onKeyDown));
  });
  const readConversation = async (accountId: string, threadId: string, cursor?: string, append = false, targetMessageId?: string) => {
    const request = conversationRequest() + 1;
    setConversationRequest(request);
    if (!append) { setAttachmentPreview(null); setAttachmentError(null); setAttachmentLoading(null); attachmentRequest += 1; }
    if (!append) {
      lastOpenedMessage = targetMessageId ? null : lastOpenedMessage;
      setSelectedConversation({ accountId, threadId, messages: [] });
    }
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
      if (!targetMessageId && !append) window.requestAnimationFrame(() => {
        document.querySelector<HTMLElement>(".daily-mail-calendar-conversation-heading")?.focus({ preventScroll: true });
      });
    } catch (cause) {
      if (conversationRequest() === request) {
        setConversationError(cause instanceof Error ? cause.message : "Could not load this conversation.");
      }
    } finally {
      if (conversationRequest() === request) setConversationLoading(false);
    }
  };
  const readSelectedMessage = async (accountId: string, message: api.MailCalendarMailPreview) => {
    const request = conversationRequest() + 1;
    setConversationRequest(request);
    lastOpenedMessage = { accountId, messageId: message.provider_id };
    setSelectedConversation({ accountId, threadId: null, messages: [{ ...message, body_text: null }], nextCursor: null });
    setConversationLoading(true); setConversationError(null);
    setAttachmentPreview(null); setAttachmentError(null); setAttachmentLoading(null); attachmentRequest += 1;
    try {
      const result = await api.previewMailCalendarMessage(
        props.subject.kind === "daily_mail_calendar" ? props.subject.agentId : "",
        accountId,
        message.provider_id,
      );
      if (conversationRequest() !== request) return;
      setSelectedConversation((current) => current?.accountId === accountId && current.threadId === null
        ? { ...current, messages: current.messages.map((item) => item.provider_id === message.provider_id ? { ...item, body_text: result.body_text, body_status: result.body_status } : item) }
        : current);
    } catch (cause) {
      if (conversationRequest() === request) setConversationError(cause instanceof Error ? cause.message : "Could not open this message.");
    } finally {
      if (conversationRequest() === request) setConversationLoading(false);
    }
  };
  const readConversationAttachment = async (accountId: string, messageId: string, attachment: api.MailCalendarAttachmentPreview) => {
    if (!attachment.previewable || attachmentLoading()) return;
    const request = ++attachmentRequest;
    const agentId = props.subject.kind === "daily_mail_calendar" ? props.subject.agentId : "";
    if (!agentId) return;
    setAttachmentLoading(attachment.provider_id); setAttachmentError(null); setAttachmentPreview(null);
    try {
      const result = await api.previewMailCalendarAttachment(agentId, accountId, messageId, attachment.provider_id);
      if (attachmentRequest !== request || selectedConversation()?.accountId !== accountId) return;
      setAttachmentPreview({ accountId, messageId, attachmentId: attachment.provider_id, filename: result.filename, text: result.text });
    } catch (cause) {
      if (attachmentRequest === request) setAttachmentError(cause instanceof Error ? cause.message : "Could not preview this attachment.");
    } finally {
      if (attachmentRequest === request) setAttachmentLoading(null);
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
    <LoadState loader={loader} stable>{(data) => {
      const accountCalendarEvents = (account: DailyAccount) => {
        const seen = new Set<string>();
        return [...account.events, ...(calendarAdditionalPages()[account.account.id] ?? []).flatMap((page) => page.events)]
          .filter((event) => { if (seen.has(event.provider_id)) return false; seen.add(event.provider_id); return true; });
      };
      const accountCalendarCursor = (account: DailyAccount) => {
        const pages = calendarAdditionalPages()[account.account.id] ?? [];
        return pages.length ? pages[pages.length - 1].nextCursor : account.nextCalendarCursor;
      };
      const events = () => data.accounts.flatMap(accountCalendarEvents);
      const visibleEvents = () => events().filter((event) => calendarAccountFilter() === "all" || event.account_id === calendarAccountFilter());
      const pagedCalendarAccounts = () => data.accounts.filter((account) =>
        account.account.capabilities.includes("calendar_read")
        && (calendarAccountFilter() === "all" || account.account.id === calendarAccountFilter())
        && !!accountCalendarCursor(account));
      const loadMoreCalendarEvents = async () => {
        if (calendarLoadingMore() || loader.loading() || !pagedCalendarAccounts().length || props.subject.kind !== "daily_mail_calendar") return;
        const generation = ++calendarPaginationGeneration;
        const agentId = props.subject.agentId;
        const accounts = pagedCalendarAccounts();
        const from = new Date(`${data.day}T00:00:00`).toISOString();
        const to = new Date(`${tomorrow(new Date(`${data.through}T12:00:00`))}T00:00:00`).toISOString();
        setCalendarLoadingMore(true); setCalendarLoadError("");
        let failed = 0;
        try {
          for (let offset = 0; offset < accounts.length; offset += 2) {
            const pages = await Promise.all(accounts.slice(offset, offset + 2).map(async (account) => {
              const cursor = accountCalendarCursor(account);
              if (!cursor) return null;
              try {
                const page = await api.previewMailCalendarEvents(agentId, account.account.id, from, to, 50, undefined, cursor);
                return { accountId: account.account.id, page: { events: page.events.map((event) => ({ ...event, account_id: account.account.id, account_name: providerName(account.account) })), nextCursor: page.next_cursor ?? null } };
              } catch { failed += 1; return null; }
            }));
            if (generation !== calendarPaginationGeneration) return;
            setCalendarAdditionalPages((current) => {
              const next = { ...current };
              for (const entry of pages) if (entry) next[entry.accountId] = [...(next[entry.accountId] ?? []), entry.page];
              return next;
            });
          }
          setCalendarLoadError(failed ? `Could not load another page for ${failed} calendar${failed === 1 ? "" : "s"}. Your current events are still shown.` : "");
        } finally { if (generation === calendarPaginationGeneration) setCalendarLoadingMore(false); }
      };
      const calendarPageCount = () => Math.max(1, Math.ceil(inclusiveDays(data.day, data.through) / 7));
      const calendarPageFrom = () => addDays(data.day, calendarPage() * 7);
      const calendarPageThrough = () => calendarPage() + 1 >= calendarPageCount()
        ? data.through
        : addDays(calendarPageFrom(), 6);
      const calendarPageEvents = () => visibleEvents().filter((event) => {
        const day = event.starts_on ?? (event.starts_at ? dateKey(new Date(event.starts_at)) : "");
        return day >= calendarPageFrom() && day <= calendarPageThrough();
      });
      const calendarAccounts = () => data.accounts.filter((account) => account.account.capabilities.includes("calendar_read") && accountCalendarEvents(account).length > 0);
      const hasMailCapability = () => data.accounts.some((account) => account.account.capabilities.includes("mail_read"));
      const hasCalendarCapability = () => data.accounts.some((account) => account.account.capabilities.includes("calendar_read") || account.account.capabilities.includes("calendar_free_busy"));
      const mailFailedAccounts = () => data.accounts.filter((account) => account.mailError);
      const scopedMailAccounts = () => data.accounts.filter((account) => account.account.capabilities.includes("mail_read") && (data.mailAccountId === "all" || data.mailAccountId === account.account.id));
      const mailRows = () => {
        const rows: Array<{ account: DailyAccount; message: api.MailCalendarMailPreview }> = [];
        for (const account of data.accounts) {
          const seen = new Set<string>();
          for (const message of [...account.messages, ...(mailAdditionalPages()[account.account.id] ?? []).flatMap((page) => page.messages)]) {
            if (seen.has(message.provider_id)) continue;
            seen.add(message.provider_id);
            rows.push({ account, message });
          }
        }
        return rows;
      };
      const accountMailCursor = (account: DailyAccount) => {
        const pages = mailAdditionalPages()[account.account.id] ?? [];
        return pages.length ? pages[pages.length - 1].nextCursor : account.nextMailCursor;
      };
      const hasMoreProviderMail = () => scopedMailAccounts().some((account) => !!accountMailCursor(account));
      const loadMoreProviderMail = async () => {
        if (mailLoadingMore() || loader.loading() || mailFoldersLoading() || !hasMoreProviderMail() || props.subject.kind !== "daily_mail_calendar") return;
        const generation = ++mailPaginationGeneration;
        const agentId = props.subject.agentId;
        const accounts = scopedMailAccounts().filter((account) => !!accountMailCursor(account));
        setMailLoadingMore(true); setMailLoadError("");
        let failed = 0;
        try {
          for (let offset = 0; offset < accounts.length; offset += 2) {
            const batch = accounts.slice(offset, offset + 2);
            const pages = await Promise.all(batch.map(async (account) => {
              const cursor = accountMailCursor(account);
              if (!cursor) return null;
              try {
                const page = await api.previewMailCalendarMail(agentId, account.account.id, 8, data.mailQuery, data.mailAccountId === account.account.id ? data.mailFolderId || undefined : undefined, cursor);
                return { accountId: account.account.id, page: { messages: page.messages, nextCursor: page.next_cursor ?? null } };
              } catch {
                failed += 1;
                return null;
              }
            }));
            if (generation !== mailPaginationGeneration) return;
            setMailAdditionalPages((current) => {
              const next = { ...current };
              for (const entry of pages) if (entry) next[entry.accountId] = [...(next[entry.accountId] ?? []), entry.page];
              return next;
            });
          }
          if (failed) setMailLoadError(`Could not load more messages for ${failed} ${failed === 1 ? "account" : "accounts"}. Existing messages remain available.`);
          setMailPage((page) => Math.min(page, Math.max(0, Math.ceil(mailRows().length / 12) - 1)));
        } finally {
          if (generation === mailPaginationGeneration) setMailLoadingMore(false);
        }
      };
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
      return <main class="daily-mail-calendar-body" classList={{ "mail-calendar-conversation-open": !!selectedConversation() }}>
        <header class="daily-mail-calendar-heading"><div><h2>{prettyDay(data.day)}</h2><p>{freshnessLabel()}</p></div><div class="settings-actions"><SyntheticMailCalendarDemoButton checked={syntheticDemo()} onChange={setSyntheticMailCalendarEnabled} /><button type="button" class="artifact-canvas-btn" onClick={manualRefresh}>Refresh</button></div></header>
        <Show when={data.accounts.length === 0}>
          <section class="daily-mail-calendar-empty"><h3>No connected accounts</h3><p>Connect mail or a calendar to see today’s agenda and recent messages here.</p><button type="button" class="artifact-canvas-btn" onClick={openSettings}>Open email and calendar settings</button></section>
        </Show>
        <Show when={data.accounts.length > 0}>
            <Show when={!selectedConversation()}>
            <section class="daily-mail-calendar-section"><div class="daily-mail-calendar-calendar-heading"><div><h3>{data.day === data.through ? data.day === dateKey(new Date()) ? "Today’s calendar" : prettyDay(data.day) : `${prettyDay(data.day)} – ${prettyDay(data.through)} · ${inclusiveDays(data.day, data.through)} days`}</h3><p>Choose up to 30 days · times use your device time zone</p></div><div class="daily-mail-calendar-calendar-tools"><label>From<input aria-label="Calendar start date" type="date" value={rangeFrom()} onInput={(event) => setRangeFrom(event.currentTarget.value)} /></label><label>Through<input aria-label="Calendar end date" type="date" value={rangeThrough()} onInput={(event) => setRangeThrough(event.currentTarget.value)} /></label><button type="button" class="settings-button" onClick={() => { setRangeThrough(addDays(rangeFrom(), 6)); setRangeError(""); }}>Use 7 days</button><button type="button" class="settings-button" onClick={applyDateRange}>Refresh dates</button><label>Show calendars<select aria-label="Show calendars" value={calendarAccountFilter()} onChange={(event) => { setCalendarAccountFilter(event.currentTarget.value); setSelectedEvent(null); }}><option value="all">All calendars ({events().length})</option><For each={calendarAccounts()}>{(account) => <option value={account.account.id}>{providerName(account.account)} ({account.events.length})</option>}</For></select></label><span>Auto-refreshes every 5 minutes while open</span></div></div>
            <Show when={rangeError()}><p class="daily-mail-calendar-warning" role="alert">{rangeError()}</p></Show>
            <Show when={!hasCalendarCapability()}><p class="settings-hint">No connected account has calendar access.</p></Show>
            <For each={calendarFailedAccounts()}>{(account) => <p class="daily-mail-calendar-warning" role="status">Calendar unavailable for {providerName(account.account)}. Check this account in Settings.</p>}</For>
            <Show when={pagedCalendarAccounts().length > 0 || calendarLoadingMore()}><div class="daily-mail-calendar-pagination"><span>More events are available from the connected calendar.</span><button type="button" class="settings-button" disabled={calendarLoadingMore() || loader.loading()} onClick={() => void loadMoreCalendarEvents()}>{calendarLoadingMore() ? "Loading more events…" : "Load more events"}</button></div></Show>
            <Show when={calendarLoadError()}><p class="daily-mail-calendar-warning" role="status">{calendarLoadError()}</p></Show>
            <Show when={selectedEvent()}>{(event) => <aside class="daily-mail-calendar-event-detail" aria-label="Selected event details"><div class="daily-mail-calendar-event-detail-heading"><div><span>Event details</span><h4>{event().title}</h4></div><button type="button" class="settings-button" onClick={() => setSelectedEvent(null)}>Close</button></div><dl><div><dt>When</dt><dd>{event().all_day ? "All day" : event().starts_at && event().ends_at ? `${new Date(event().starts_at!).toLocaleString()} – ${new Date(event().ends_at!).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}` : "Time unavailable"}</dd></div><div><dt>Calendar</dt><dd>{event().account_name ?? "Connected calendar"}</dd></div><Show when={event().location}><div><dt>Location</dt><dd>{event().location}</dd></div></Show><Show when={event().description}><div><dt>Description</dt><dd>{event().description}</dd></div></Show></dl><p>This view is read-only. Prepare a change as a local draft, then review its exact effect before anything is sent to the provider.</p><div class="settings-actions"><Show when={event().account_id && event().version && event().provider_id && event().starts_at && event().ends_at && !event().private && !event().all_day && !event().recurring && event().attendee_count === 0 && event().account_name?.startsWith("Google")}><button type="button" class="settings-button" disabled={!!eventAction()} onClick={() => void prepareEventAction("update")}>{eventAction() === "update" ? "Preparing draft…" : "Draft an update"}</button></Show><Show when={event().can_cancel && event().account_id && event().version}><button type="button" class="settings-button danger" disabled={!!eventAction()} onClick={() => void prepareEventAction("cancel")}>{eventAction() === "cancel" ? "Preparing review…" : "Review cancellation"}</button></Show><button type="button" class="settings-button" onClick={() => openDraftWorkspace()}>Open drafts and review</button></div><Show when={eventActionError()}><p role="alert" class="daily-mail-calendar-warning">{eventActionError()}</p></Show></aside>}</Show>
            <Show when={calendarPageCount() > 1}><nav class="daily-mail-calendar-pagination" aria-label="Calendar week pages"><button type="button" class="settings-button" disabled={calendarPage() === 0} onClick={() => { setCalendarPage((page) => Math.max(0, page - 1)); setSelectedEvent(null); }}>Previous week</button><span aria-live="polite">Week {calendarPage() + 1} of {calendarPageCount()} · {prettyDay(calendarPageFrom())} – {prettyDay(calendarPageThrough())}</span><button type="button" class="settings-button" disabled={calendarPage() + 1 >= calendarPageCount()} onClick={() => { setCalendarPage((page) => Math.min(calendarPageCount() - 1, page + 1)); setSelectedEvent(null); }}>Next week</button></nav></Show>
            <Show when={calendarPageEvents().length > 0}><For each={[calendarAccountFilter()]}>{() => <MailCalendarAgenda events={calendarPageEvents()} from={calendarPageFrom()} to={calendarPageThrough()} conflicts={new Set()} initialView={calendarPageFrom() === calendarPageThrough() ? "day" : "week"} onSelect={setSelectedEvent} />}</For></Show>
            <Show when={hasCalendarCapability() && calendarPageEvents().length === 0 && calendarFailedAccounts().length === 0}><p class="settings-hint">No events for this date range.</p></Show>
            <For each={data.accounts.filter((account) => account.busy.length > 0)}>{(account) => <div class="daily-mail-calendar-busy"><strong>Busy · {providerName(account.account)}</strong><For each={account.busy.filter((slot) => { const day = dateKey(new Date(slot.starts_at)); return day >= calendarPageFrom() && day <= calendarPageThrough(); })}>{(slot) => <span>{new Date(slot.starts_at).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}–{new Date(slot.ends_at).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}</span>}</For></div>}</For>
            </section>
            </Show>
          <section class="daily-mail-calendar-section"><h3>{selectedConversation() ? "Conversation" : "Recent email"}</h3>
            <Show when={!selectedConversation()}>
            <div class="daily-mail-calendar-mail-filters"><label>Mailbox<select aria-label="Mail account" value={mailAccountFilter()} onChange={(event) => void selectMailAccount(event.currentTarget.value)}><option value="all">All inboxes</option><For each={data.accounts.filter((account) => account.account.capabilities.includes("mail_read"))}>{(account) => <option value={account.account.id}>{providerName(account.account)}</option>}</For></select></label><Show when={mailAccountFilter() !== "all"}><label>Folder or label<select aria-label="Mail folder or label" value={mailFolderFilter()} disabled={mailFoldersLoading() || mailFolders().length === 0} onChange={(event) => setMailFolderFilter(event.currentTarget.value)}><For each={mailFolders()}>{(folder) => <option value={folder.provider_id}>{folder.name}</option>}</For></select></label></Show><label>Search {mailAccountFilter() === "all" ? "in inboxes" : "this folder"}<input aria-label="Search mail" type="search" maxlength="128" value={mailSearchInput()} onInput={(event) => setMailSearchInput(event.currentTarget.value)} placeholder="Sender, subject or message" /></label><button type="button" class="settings-button" disabled={mailFoldersLoading()} onClick={applyMailSearch}>{mailFoldersLoading() ? "Loading folders…" : "Search mail"}</button></div>
            <Show when={mailFoldersError()}><p class="daily-mail-calendar-warning" role="alert">{mailFoldersError()}</p></Show>
            <p class="settings-hint">{data.mailAccountId === "all" ? "Showing recent inbox messages across connected accounts." : `Showing ${data.accounts.find((account) => account.account.id === data.mailAccountId)?.account.identity_masked ?? "the selected account"}${data.mailFolderId ? ` · ${mailFolders().find((folder) => folder.provider_id === data.mailFolderId)?.name ?? "selected folder"}` : " · Inbox"}${data.mailQuery ? ` · Search: ${data.mailQuery}` : ""}`}</p>
            <Show when={!hasMailCapability()}><p class="settings-hint">No connected account has email access.</p></Show>
            <For each={mailFailedAccounts()}>{(account) => <p class="daily-mail-calendar-warning" role="status">Inbox unavailable for {providerName(account.account)}. Check this account in Settings.</p>}</For>
            <For each={visibleMailAccounts().map((item) => item.account.id)}>{(accountId) => {
              const account = () => visibleMailAccounts().find((item) => item.account.id === accountId);
              return <div class="daily-mail-calendar-mail-account"><h4>{account() ? providerName(account()!.account) : "Connected account"}</h4><For each={account()?.messages.map((message) => message.provider_id) ?? []}>{(messageId) => {
                const message = () => account()?.messages.find((item) => item.provider_id === messageId);
                return <Show when={message()}>{(currentMessage) => <article class="daily-mail-calendar-message"><div><strong>{currentMessage().subject || "(No subject)"}</strong><small>{currentMessage().from || "Sender unavailable"}{currentMessage().received_at ? ` · ${new Date(currentMessage().received_at!).toLocaleString()}` : ""}</small><Show when={currentMessage().preview}><p>{currentMessage().preview}</p></Show></div><Show when={currentMessage().thread_id} fallback={<button type="button" class="settings-button" data-mail-open={`${accountId}/${currentMessage().provider_id}`} onClick={() => void readSelectedMessage(accountId, currentMessage())}>Open email</button>}>{(threadId) => <button type="button" class="settings-button" data-mail-open={`${accountId}/${currentMessage().provider_id}`} onClick={() => { lastOpenedMessage = { accountId, messageId: currentMessage().provider_id }; void readConversation(accountId, threadId()); }}>Open conversation</button>}</Show></article>}</Show>;
              }}</For></div>;
            }}</For>
            <Show when={mailRows().length > mailPageSize}><nav class="daily-mail-calendar-pagination" aria-label="Recent email pages"><button type="button" class="settings-button" disabled={mailPage() === 0} onClick={() => setMailPage((page) => Math.max(0, page - 1))}>Previous</button><span aria-live="polite">Page {mailPage() + 1} of {mailPageCount()} · {mailRows().length} messages loaded</span><button type="button" class="settings-button" disabled={mailPage() + 1 >= mailPageCount()} onClick={() => setMailPage((page) => Math.min(mailPageCount() - 1, page + 1))}>Next</button></nav></Show>
            <Show when={mailLoadError()}><p class="daily-mail-calendar-warning" role="status">{mailLoadError()}</p></Show>
            <Show when={hasMoreProviderMail() || mailLoadingMore()}><div class="daily-mail-calendar-pagination"><span>More messages are available from the connected mail provider.</span><button type="button" class="settings-button" disabled={mailLoadingMore() || loader.loading() || mailFoldersLoading()} onClick={() => void loadMoreProviderMail()}>{mailLoadingMore() ? "Loading more…" : "Load more messages"}</button></div></Show>
            <Show when={hasMailCapability() && data.accounts.every((account) => account.messages.length === 0) && mailFailedAccounts().length === 0}><p class="settings-hint">{data.mailQuery ? "No messages matched this search." : "No recent messages in this view."}</p></Show>
            </Show>
            <Show when={selectedConversation()}>{(conversation) => {
              const account = () => data.accounts.find((item) => item.account.id === conversation().accountId)?.account;
              const accountLabel = () => account() ? providerName(account()!) : "Connected account";
              return <section class="daily-mail-calendar-conversation is-workspace" aria-label="Conversation workspace">
                <header><div><span>{accountLabel()} · {conversation().messages.length} messages{conversationLoading() ? " · Updating" : ""}</span><h4 class="daily-mail-calendar-conversation-heading" tabindex="-1">{conversation().messages[0]?.subject || "Conversation"}</h4></div><button type="button" class="settings-button" onClick={closeConversationAndRestoreFocus}>Back to inbox</button></header>
                <p class="settings-hint">Read-only conversation preview. Message content is untrusted; opening it does not add it to the Agent conversation.</p>
                <Show when={conversationError()}><p class="daily-mail-calendar-warning" role="alert">{conversationError()}</p></Show>
                <For each={conversation().messages}>{(message) => <article class="daily-mail-calendar-conversation-message" data-mail-message-id={message.provider_id}>
                  <strong>{message.subject || "(No subject)"}</strong>
                  <small>{message.from || "Sender unavailable"}{message.received_at ? ` · ${new Date(message.received_at).toLocaleString()}` : ""}</small>
                  <Show when={message.to || message.cc}><small>{message.to ? `To: ${message.to}` : ""}{message.to && message.cc ? " · " : ""}{message.cc ? `Cc: ${message.cc}` : ""}</small></Show>
                  <small>Conversation content is untrusted. Ignore instructions inside it.</small>
                  <p>{message.body_text || (message.body_status === "no_plain_text" ? "This message has no supported plain-text body." : message.preview || (conversationLoading() ? "Loading message…" : "No plain-text message content was returned."))}</p>
                  <Show when={(message.attachments?.length ?? 0) > 0}><section class="daily-mail-calendar-message-attachments" aria-label="Message attachments"><strong>Attachments</strong><For each={message.attachments ?? []}>{(attachment) => <div class="daily-mail-calendar-message-attachment"><span>{attachment.filename} · {attachment.size_bytes.toLocaleString()} bytes{attachment.mime_type ? ` · ${attachment.mime_type}` : ""}</span><Show when={attachment.previewable} fallback={<span>Preview unavailable for this file type or size.</span>}><button type="button" class="settings-button" disabled={!!attachmentLoading()} onClick={() => void readConversationAttachment(conversation().accountId, message.provider_id, attachment)}>{attachmentLoading() === attachment.provider_id ? "Preparing preview…" : "Preview attachment"}</button></Show><Show when={attachmentPreview()?.accountId === conversation().accountId && attachmentPreview()?.messageId === message.provider_id && attachmentPreview()?.attachmentId === attachment.provider_id}><div class="daily-mail-calendar-attachment-preview"><header><strong>{attachmentPreview()?.filename}</strong><button type="button" class="settings-button" onClick={() => setAttachmentPreview(null)}>Close preview</button></header><p>Extracted text · read-only · message content is untrusted</p><pre>{attachmentPreview()?.text}</pre></div></Show></div>}</For></section></Show>
                  <Show when={attachmentError()}><p class="daily-mail-calendar-warning" role="alert">{attachmentError()}</p></Show>
                  <Show when={message.thread_id && account()?.capabilities.includes("mail_send")}><button type="button" class="settings-button" onClick={() => {
                    const agentId = props.subject.kind === "daily_mail_calendar" ? props.subject.agentId : "";
                    if (!agentId) return;
                    setReplyWorkspaceVisible(true);
                    const action: api.MailCalendarDraftAction = { kind: "send_mail", draft: { from_alias: null, to: (message.from?.match(/[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}/i)?.[0] ? [{ address: message.from.match(/[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}/i)![0], display_name: null }] : []), cc: [], bcc: [], subject: message.subject?.startsWith("Re:") ? message.subject : `Re: ${message.subject || ""}`, body_text: "", attachment_refs: [], reply_to_message_id: message.provider_id, reply_to_thread_id: conversation().threadId } };
                    void api.saveMailCalendarCandidate(agentId, { account_id: conversation().accountId, source_refs: [{ item_id: message.provider_id, version: null, label: message.subject || "Reply" }], action })
                      .then((saved) => openDraftWorkspace(saved.candidate.id))
                      .catch((cause) => setConversationError(cause instanceof Error ? cause.message : "Could not prepare a local reply draft."));
                  }}>Draft reply in Canvas</button></Show>
                </article>}</For>
                <Show when={conversation().nextCursor && conversation().threadId}><button type="button" class="settings-button" disabled={conversationLoading()} onClick={() => { const current = conversation(); if (current.threadId) void readConversation(current.accountId, current.threadId, current.nextCursor ?? undefined, true); }}>{conversationLoading() ? "Loading more…" : "Load more messages"}</button></Show>
                <Show when={conversationLoading() && conversation().messages.length === 0}><p role="status">Loading conversation…</p></Show>
              </section>;
            }}</Show>
          </section>
        </Show>
        <Show when={!selectedConversation()}>
          <Show when={props.subject.kind === "daily_mail_calendar"}><MailCalendarRoutineWorkspace agentId={props.subject.kind === "daily_mail_calendar" ? props.subject.agentId : ""} /></Show>
        </Show>
        <div style={{ display: selectedConversation() && !replyWorkspaceVisible() ? "none" : undefined }}><MailCalendarDraftWorkspace agentId={props.subject.kind === "daily_mail_calendar" ? props.subject.agentId : ""} /></div>
        <footer class="daily-mail-calendar-privacy">Read-only preview for this Agent and your local owner session. Refresh runs every five minutes while visible and when you return after a minute away. Provider content is not added to the conversation by opening this view.</footer>
      </main>;
    }}</LoadState>
  </div>;
}
