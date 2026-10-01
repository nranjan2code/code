import { createEffect, createSignal, For, Show, untrack } from "solid-js";
import * as api from "../../api";
import type { MailCalendarCandidate, MailCalendarDraftAction, MailCalendarMailPreview, MailCalendarAccount } from "../../api";
import ConfirmModal from "../ConfirmModal";
import type { ConfirmConfig } from "../ConfirmModal";

const LOCAL_DRAFT = "local-draft";
const PAGE_SIZE = 8;
const parseAddresses = (value: string) => value.split(/[;,]/).map((part) => part.trim()).filter(Boolean).map((address) => ({ address, display_name: null }));
const formatAddresses = (addresses: Array<{ address: string; display_name: string | null }>) => addresses.map(({ address, display_name }) => display_name ? `${display_name} <${address}>` : address).join(", ") || "None";
const localInput = (date: Date) => new Date(date.getTime() - date.getTimezoneOffset() * 60_000).toISOString().slice(0, 16);
const actionName = (action: MailCalendarDraftAction) => action.kind === "send_mail" ? "Email" : action.kind === "cancel_event" ? "Event cancellation" : "Calendar event";
const actionVerb = (action: MailCalendarDraftAction) => action.kind === "send_mail" ? "send" : action.kind === "cancel_event" ? "cancel" : action.kind === "update_event" ? "update" : "create";

export default function MailCalendarDraftWorkspace(props: { agentId: string; openCandidate?: api.MailCalendarCandidate | null; focusReply?: boolean; inlineReply?: boolean; onCloseInlineReply?: () => void; onCandidateOpened?: (candidate: api.MailCalendarCandidate) => void }) {
  const [expanded, setExpanded] = createSignal(false);
  const [accounts, setAccounts] = createSignal<MailCalendarAccount[]>([]);
  const [candidates, setCandidates] = createSignal<MailCalendarCandidate[]>([]);
  const [loading, setLoading] = createSignal(true);
  const [busy, setBusy] = createSignal(false);
  const [note, setNote] = createSignal("");
  const [error, setError] = createSignal("");
  const [revisionConflict, setRevisionConflict] = createSignal(false);
  const [page, setPage] = createSignal(0);
  const [selected, setSelected] = createSignal<MailCalendarCandidate | null>(null);
  const [accountId, setAccountId] = createSignal(LOCAL_DRAFT);
  const [to, setTo] = createSignal("");
  const [cc, setCc] = createSignal("");
  const [bcc, setBcc] = createSignal("");
  const [subject, setSubject] = createSignal("");
  const [body, setBody] = createSignal("");
  const [title, setTitle] = createSignal("");
  const [description, setDescription] = createSignal("");
  const [location, setLocation] = createSignal("");
  const [starts, setStarts] = createSignal("");
  const [ends, setEnds] = createSignal("");
  const [review, setReview] = createSignal<ConfirmConfig | null>(null);
  let loadGeneration = 0;
  let replyMessageField: HTMLTextAreaElement | undefined;

  const refresh = async (agentId = props.agentId) => {
    if (!agentId) return;
    const generation = ++loadGeneration;
    setLoading(candidates().length === 0);
    try {
      const [inventory, saved] = await Promise.all([
        api.listMailCalendarAccounts(agentId),
        api.listMailCalendarCandidates(agentId),
      ]);
      if (generation !== loadGeneration || agentId !== props.agentId) return;
      setAccounts(inventory.accounts);
      setCandidates(saved.candidates.slice().sort((a, b) => b.created_at.localeCompare(a.created_at)));
      setError("");
    } catch (cause) {
      if (generation === loadGeneration) setError(cause instanceof Error ? cause.message : "Could not load this Agent’s saved drafts.");
    } finally {
      if (generation === loadGeneration) setLoading(false);
    }
  };

  createEffect(() => {
    const id = props.agentId;
    untrack(() => {
      setSelected(null);
      setPage(0);
      void refresh(id);
    });
  });

  const openCandidate = (candidate: MailCalendarCandidate) => {
    setSelected(candidate);
    setRevisionConflict(false);
    setAccountId(candidate.account_id);
    setError("");
    if (candidate.action.kind === "send_mail") {
      setTo(candidate.action.draft.to.map((item) => item.address).join(", "));
      setCc(candidate.action.draft.cc.map((item) => item.address).join(", "));
      setBcc(candidate.action.draft.bcc.map((item) => item.address).join(", "));
      setSubject(candidate.action.draft.subject);
      setBody(candidate.action.draft.body_text);
      setTitle(""); setDescription(""); setLocation(""); setStarts(""); setEnds("");
    } else if (candidate.action.kind === "create_event" || candidate.action.kind === "update_event") {
      const event = candidate.action.draft;
      setTitle(event.title); setDescription(event.description); setLocation(event.location ?? "");
      setStarts(localInput(new Date(event.starts_at))); setEnds(localInput(new Date(event.ends_at)));
      setTo(""); setCc(""); setBcc(""); setSubject(""); setBody("");
    } else {
      setTo(""); setCc(""); setBcc(""); setSubject(""); setBody("");
      setTitle(""); setDescription(""); setLocation(""); setStarts(""); setEnds("");
    }
  };

  const startDraft = (kind: "mail" | "calendar") => {
    setError(""); setNote(""); setSelected(null);
    if (kind === "mail") {
      setTo(""); setCc(""); setBcc(""); setSubject(""); setBody("");
      setTitle(""); setDescription(""); setLocation(""); setStarts(""); setEnds("");
    } else {
      const start = new Date(Date.now() + 60 * 60_000); start.setMinutes(0, 0, 0);
      setStarts(localInput(start)); setEnds(localInput(new Date(start.getTime() + 60 * 60_000)));
      setTitle(""); setDescription(""); setLocation("");
      setTo(""); setCc(""); setBcc(""); setSubject(""); setBody("");
    }
    setExpanded(true);
  };

  const reconcileCalendarEvent = async (candidate: MailCalendarCandidate) => {
    if ((candidate.action.kind !== "create_event" && candidate.action.kind !== "update_event") || !candidate.candidate_digest) return;
    setBusy(true); setError(""); setNote("");
    try {
      const result = await api.reconcileMailCalendarEventCandidate(props.agentId, candidate.id, candidate.revision, candidate.candidate_digest);
      setNote(result.matched
        ? "The provider confirms this event was created. The action is now marked confirmed."
        : "No matching event is visible yet. The outcome remains unknown, and this action cannot be retried.");
      await refresh(props.agentId);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Could not check the provider event result.");
    } finally { setBusy(false); }
  };

  const reconcileMailSend = async (candidate: MailCalendarCandidate) => {
    if (candidate.action.kind !== "send_mail" || !candidate.candidate_digest) return;
    setBusy(true); setError(""); setNote("");
    try {
      const result = await api.reconcileMailCalendarMailCandidate(props.agentId, candidate.id, candidate.revision, candidate.candidate_digest);
      setNote(result.matched
        ? "The provider confirms this email was sent. The action is now marked confirmed."
        : "No matching sent email is visible yet. The outcome remains unknown, and this action cannot be retried.");
      await refresh(props.agentId);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Could not check the provider send result.");
    } finally { setBusy(false); }
  };

  const actionForEditor = (): MailCalendarDraftAction | null => {
    if (selected()?.action.kind === "cancel_event") return selected()!.action;
    const current = selected()?.action;
    if (current?.kind === "send_mail" || (!current && editorKind() === "mail")) {
      return { kind: "send_mail", draft: {
        from_alias: null, to: parseAddresses(to()), cc: parseAddresses(cc()), bcc: parseAddresses(bcc()),
        subject: subject(), body_text: body(), attachment_refs: [],
        reply_to_message_id: current?.kind === "send_mail" ? current.draft.reply_to_message_id : null,
        reply_to_thread_id: current?.kind === "send_mail" ? current.draft.reply_to_thread_id : null,
      } };
    }
    const start = new Date(starts()); const end = new Date(ends());
    if (!Number.isFinite(start.getTime()) || !Number.isFinite(end.getTime())) return null;
    const draft = { title: title(), description: description(), location: location().trim() || null,
      starts_at: start.toISOString(), ends_at: end.toISOString(), time_zone: Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC",
      all_day: false, attendee_addresses: [], recurrence: null, occurrence_id: null };
    if (current?.kind === "update_event") return { kind: "update_event", event_id: current.event_id, source_version: current.source_version, draft };
    return { kind: "create_event", draft };
  };

  const [editorKind, setEditorKind] = createSignal<"mail" | "calendar" | null>(null);
  const selectedReply = () => {
    const current = selected();
    return current?.action.kind === "send_mail" && !!current.action.draft.reply_to_message_id;
  };
  const newMail = () => { setEditorKind("mail"); startDraft("mail"); };
  const newEvent = () => { setEditorKind("calendar"); startDraft("calendar"); };
  const beginOpen = (candidate: MailCalendarCandidate) => {
    setEditorKind(candidate.action.kind === "send_mail" ? "mail" : candidate.action.kind === "cancel_event" ? null : "calendar");
    openCandidate(candidate);
  };

  const buildReview = async (candidate: MailCalendarCandidate) => {
    const action = candidate.action;
    const account = accounts().find((item) => item.id === candidate.account_id);
    if (!candidate.candidate_digest || candidate.action_state || candidate.account_id === LOCAL_DRAFT) {
      setError("This draft is not eligible for a provider action. Save a current, account-bound revision before review."); return;
    }
    const canAct = account?.status === "connected" && !!account.credential_available && !account.revoked_at;
    if (!account || !canAct) { setError("This account is not connected. Reconnect it before reviewing a provider action."); return; }
    if (action.kind === "send_mail" && (!account.capabilities.includes("mail_send") || (action.draft.reply_to_message_id && !account.capabilities.includes("mail_read")))) {
      setError("This account no longer has the access required to send this message."); return;
    }
    if ((action.kind === "create_event" || action.kind === "update_event" || action.kind === "cancel_event") && !account.capabilities.includes("calendar_write")) {
      setError("This account no longer has calendar-write access."); return;
    }
    if ((action.kind === "update_event" || action.kind === "cancel_event") && account.provider !== "google") {
      setError("Calendar updates and cancellations are currently supported only for Google events."); return;
    }
    if (action.kind === "update_event" && (action.draft.all_day || action.draft.attendee_addresses.length || action.draft.recurrence || action.draft.occurrence_id)) {
      setError("This update is outside the supported single, timed event boundary."); return;
    }
    if (action.kind === "cancel_event" && (action.occurrence_id || action.whole_series)) {
      setError("Only one standalone event can be cancelled; occurrences and series are not supported."); return;
    }

    let mailContext: api.MailCalendarReviewContext | null = null;
    if (action.kind === "send_mail") {
      setBusy(true); setError("");
      try {
        mailContext = await api.getMailCalendarReviewContext(props.agentId, candidate.id, candidate.revision, candidate.candidate_digest!);
      } catch (cause) {
        setError(cause instanceof Error ? cause.message : "The sender and reply source could not be verified. No provider action is ready for review.");
        return;
      } finally {
        setBusy(false);
      }
    }

    const reviewContent = <div class="mail-calendar-review-payload">
      {(() => {
        const mail = action.kind === "send_mail" ? action : null;
        const event = action.kind === "create_event" || action.kind === "update_event" ? action : null;
        return <>
      <dl><dt>Account</dt><dd>{account.provider}{account.identity_masked ? ` · ${account.identity_masked}` : ""}</dd>
        <Show when={mail}>{(value) => <><dt>From</dt><dd>{mailContext?.sender} · connected account default sender</dd><dt>Outgoing Reply-To</dt><dd>Not set</dd><Show when={value().draft.reply_to_message_id}><dt>Source From</dt><dd>{mailContext?.source_from || "Not provided by the provider"}</dd><dt>Source Reply-To</dt><dd>{mailContext?.source_reply_to || "Not provided by the provider"}</dd></Show><dt>To</dt><dd>{formatAddresses(value().draft.to)}</dd><dt>Cc</dt><dd>{formatAddresses(value().draft.cc)}</dd><dt>Bcc</dt><dd>{formatAddresses(value().draft.bcc)}</dd><dt>Subject</dt><dd>{value().draft.subject || "(no subject)"}</dd></>}</Show>
        <Show when={event}>{(value) => <><dt>Event</dt><dd>{value().draft.title}</dd><dt>Starts</dt><dd>{new Date(value().draft.starts_at).toLocaleString()}</dd><dt>Ends</dt><dd>{new Date(value().draft.ends_at).toLocaleString()}</dd><dt>Location</dt><dd>{value().draft.location || "None"}</dd><dt>Attendees</dt><dd>{value().draft.attendee_addresses.map((item) => item.address).join(", ") || "None"}</dd></>}</Show>
        <Show when={action.kind === "cancel_event"}><dt>Event</dt><dd>{candidate.source_refs[0]?.label || "Selected event"}</dd><dt>Scope</dt><dd>This event only</dd></Show>
      </dl>
      <Show when={mail}>{(value) => <><strong>Full message</strong><pre>{value().draft.body_text || "(empty message)"}</pre></>}</Show>
      <Show when={event}>{(value) => <><strong>Full description</strong><pre>{value().draft.description || "(no description)"}</pre></>}</Show>
      <p>Only saved revision {candidate.revision} will be used. The server rechecks its digest, source version, and account permission before the provider call.</p>
        </>;
      })()}
    </div>;
    const finalLabel = action.kind === "send_mail" ? "Send this exact email" : action.kind === "create_event" ? "Create this event" : action.kind === "update_event" ? "Update this event" : "Cancel this event";
    setReview({
      title: `Review ${actionVerb(action)}: ${actionName(action).toLowerCase()}`,
      description: "Check every field below. The provider change happens only after you choose the final action.",
      reviewContent, confirmLabel: finalLabel, isDanger: true,
      onConfirm: async () => {
        setBusy(true); setError(""); setNote("");
        try {
          const args = [props.agentId, candidate.id, candidate.revision, candidate.candidate_digest!] as const;
          const result = action.kind === "send_mail" ? await api.sendMailCalendarCandidate(...args)
            : action.kind === "create_event" ? await api.createMailCalendarEventCandidate(...args)
              : action.kind === "update_event" ? await api.updateMailCalendarEventCandidate(...args)
                : await api.cancelMailCalendarEventCandidate(...args);
          const state = result.receipt?.state ?? result.state ?? "unknown";
          setNote(state === "provider_accepted" ? "The provider accepted the request. This does not confirm delivery." : state === "failed" ? "The provider rejected the request. Create a fresh draft before trying again." : "The outcome is unknown. Do not retry this candidate; check the provider first.");
          await refresh(props.agentId);
        } catch (cause) {
          setError(`The action did not return a clear result. Do not retry until you check the provider: ${cause instanceof Error ? cause.message : "unknown error"}`);
        } finally { setBusy(false); }
      },
    });
  };

  const save = async (asNew = false) => {
    const action = actionForEditor();
    if (!action) { setError("Enter valid start and end times before saving this draft."); return; }
    if (action.kind === "send_mail" && accountId() !== LOCAL_DRAFT && action.draft.to.length + action.draft.cc.length + action.draft.bcc.length === 0) { setError("Add at least one recipient before saving an account-bound email draft."); return; }
    if (action.kind === "create_event" && accountId() !== LOCAL_DRAFT && !action.draft.title.trim()) { setError("Add an event title before saving an account-bound event draft."); return; }
    const previous = asNew ? null : selected(); setBusy(true); setError(""); setNote(""); setRevisionConflict(false);
    try {
      const result = await api.saveMailCalendarCandidate(props.agentId, {
        account_id: previous?.account_id ?? accountId(),
        ...(previous ? { candidate_id: previous.id, expected_revision: previous.revision } : {}),
        ...(previous ? {} : { source_refs: selected()?.source_refs ?? [] }), action,
      });
      setSelected(result.candidate); setEditorKind(result.candidate.action.kind === "send_mail" ? "mail" : "calendar");
      setNote("Saved in this Agent’s encrypted work area. Nothing was sent or changed on the provider.");
      await refresh(props.agentId);
    } catch (cause) {
      if (cause instanceof api.ApiError && cause.status === 409) {
        setRevisionConflict(true);
        setError("This draft changed elsewhere. Your edits remain here until you choose how to resolve the conflict.");
      } else setError(`Could not save this revision: ${cause instanceof Error ? cause.message : "unknown error"}`);
    } finally { setBusy(false); }
  };

  const deleteCandidate = (candidate: MailCalendarCandidate) => setReview({
    title: "Delete this saved draft?", description: "This removes the Agent work-area copy. It does not erase content already copied to conversation history.",
    confirmLabel: "Delete draft", isDanger: true,
    onConfirm: async () => {
      setBusy(true); setError("");
      try {
        await api.deleteMailCalendarCandidate(props.agentId, candidate.id, candidate.revision);
        if (selected()?.id === candidate.id) { setSelected(null); setEditorKind(null); }
        setNote("Draft deleted. No provider action was performed."); await refresh(props.agentId);
      } catch (cause) { setError(cause instanceof Error ? cause.message : "Could not delete this draft."); }
      finally { setBusy(false); }
    },
  });

  const pageCount = () => Math.max(1, Math.ceil(candidates().length / PAGE_SIZE));
  const visibleCandidates = () => candidates().slice(page() * PAGE_SIZE, (page() + 1) * PAGE_SIZE);
  const accountLabel = (id: string) => id === LOCAL_DRAFT ? "Local draft" : accounts().find((item) => item.id === id)?.identity_masked || accounts().find((item) => item.id === id)?.provider || "Account unavailable";

  createEffect(() => {
    const candidate = props.openCandidate;
    const agentId = props.agentId;
    if (!candidate || !agentId || candidate.agent_id !== agentId) return;
    setExpanded(true);
    setCandidates((current) => [candidate, ...current.filter((item) => item.id !== candidate.id)]);
    beginOpen(candidate);
    const focusReply = props.focusReply && candidate.action.kind === "send_mail" && !!candidate.action.draft.reply_to_message_id;
    if (focusReply) requestAnimationFrame(() => {
      replyMessageField?.scrollIntoView({ behavior: "smooth", block: "center" });
      replyMessageField?.focus({ preventScroll: true });
    });
    props.onCandidateOpened?.(candidate);
  });

  return <section class="daily-mail-calendar-section mail-calendar-canvas-workspace" aria-label={props.inlineReply ? "Reply composer" : "Mail and calendar drafts and review"}>
    <Show when={!props.inlineReply}><header class="daily-mail-calendar-calendar-heading"><div><h3>Drafts and Review</h3><span>Saved privately for this Agent · {candidates().length} drafts</span></div><button type="button" class="artifact-canvas-btn" aria-expanded={expanded()} onClick={() => { setExpanded((value) => !value); if (!expanded()) void refresh(); }}>{expanded() ? "Hide drafts" : "Open drafts and review"}</button></header></Show>
    <Show when={props.inlineReply && !expanded()}><p role="status" class="settings-hint">Preparing your private reply draft…</p></Show>
    <Show when={expanded()}>
      <Show when={!props.inlineReply}><p class="settings-hint">Save drafts locally first. Sending email or changing a calendar requires a separate exact Review and confirmation. Opening a draft or preview never contacts a provider with a write request.</p>
      <div class="mail-calendar-work-actions"><label>Save new draft to<select aria-label="Save new draft to" value={accountId()} onChange={(event) => setAccountId(event.currentTarget.value)}><option value={LOCAL_DRAFT}>Local draft · no provider account</option><For each={accounts().filter((account) => account.status === "connected" && account.credential_available && !account.revoked_at)}>{(account) => <option value={account.id}>{account.provider}{account.identity_masked ? ` · ${account.identity_masked}` : ""}</option>}</For></select></label><button type="button" class="settings-button" disabled={busy()} onClick={newMail}>New email draft</button><button type="button" class="settings-button" disabled={busy() || accounts().find((item) => item.id === accountId())?.provider === "apple_icloud"} onClick={newEvent}>New event draft</button><button type="button" class="settings-button" disabled={busy()} onClick={() => void refresh()}>{loading() ? "Refreshing…" : "Refresh drafts"}</button></div></Show>
      <Show when={loading()}><p role="status" class="settings-hint">Loading this Agent’s drafts…</p></Show>
      <Show when={error()}><p role="alert" class="daily-mail-calendar-warning">{error()}</p></Show>
      <Show when={note()}><p role="status" class="settings-hint">{note()}</p></Show>
      <Show when={!props.inlineReply}><Show when={candidates().length === 0 && !loading()}><p class="settings-hint">No saved drafts yet.</p></Show>
      <div class="mail-calendar-drafts"><For each={visibleCandidates()}>{(candidate) => <article class="mail-calendar-draft-row"><div><strong>{candidate.action.kind === "send_mail" ? candidate.action.draft.subject || "Email draft" : candidate.action.kind === "cancel_event" ? candidate.source_refs[0]?.label || "Event cancellation" : candidate.action.draft.title || "Calendar draft"}</strong><span>{actionName(candidate.action)} · {accountLabel(candidate.account_id)} · revision {candidate.revision}{candidate.action_state ? ` · ${candidate.action_state.replaceAll("_", " ")}` : " · local draft"}</span></div><div class="settings-actions"><Show when={(candidate.action.kind === "create_event" || candidate.action.kind === "update_event") && (candidate.action_state === "unknown" || candidate.action_state === "dispatching")}><button type="button" class="settings-button" disabled={!!busy()} onClick={() => void reconcileCalendarEvent(candidate)}>Check provider result</button></Show><Show when={candidate.action.kind === "send_mail" && (candidate.action_state === "unknown" || candidate.action_state === "dispatching")}><button type="button" class="settings-button" disabled={!!busy()} onClick={() => void reconcileMailSend(candidate)}>Check provider result</button></Show><button type="button" class="settings-button" disabled={busy() || !!candidate.action_state} onClick={() => candidate.action.kind === "cancel_event" ? buildReview(candidate) : beginOpen(candidate)}>{candidate.action.kind === "cancel_event" ? "Review cancellation" : "Open draft"}</button><button type="button" class="settings-button danger" disabled={busy() || !!candidate.action_state} onClick={() => deleteCandidate(candidate)}>Delete</button></div></article>}</For></div>
      <Show when={candidates().length > PAGE_SIZE}><nav class="daily-mail-calendar-pagination" aria-label="Draft pages"><button type="button" class="settings-button" disabled={page() === 0} onClick={() => setPage((value) => Math.max(0, value - 1))}>Previous</button><span>Page {page() + 1} of {pageCount()} · {candidates().length} drafts</span><button type="button" class="settings-button" disabled={page() + 1 >= pageCount()} onClick={() => setPage((value) => Math.min(pageCount() - 1, value + 1))}>Next</button></nav></Show></Show>
      <Show when={editorKind()}>
        <div class="mail-calendar-editor"><div class="settings-preview-heading"><strong>{selectedReply() ? "Reply draft" : editorKind() === "calendar" ? "Calendar event draft" : "Email draft"}</strong><button type="button" class="settings-button" disabled={busy()} onClick={() => { setSelected(null); setEditorKind(null); if (props.inlineReply) { setExpanded(false); props.onCloseInlineReply?.(); } }}>Close draft</button></div>
          <Show when={editorKind() === "mail"}><label>To<input type="text" value={to()} onInput={(event) => setTo(event.currentTarget.value)} /></label><label>Cc<input type="text" value={cc()} onInput={(event) => setCc(event.currentTarget.value)} /></label><label>Bcc<input type="text" value={bcc()} onInput={(event) => setBcc(event.currentTarget.value)} /></label><label>Subject<input type="text" value={subject()} onInput={(event) => setSubject(event.currentTarget.value)} /></label><label>{selectedReply() ? "Reply message" : "Message"}<textarea ref={replyMessageField} aria-label={selectedReply() ? "Reply message" : "Message"} rows={8} value={body()} onInput={(event) => setBody(event.currentTarget.value)} /></label></Show>
          <Show when={editorKind() === "calendar"}><label>Event title<input type="text" value={title()} onInput={(event) => setTitle(event.currentTarget.value)} /></label><div class="mail-calendar-work-actions"><label>Starts<input type="datetime-local" value={starts()} onInput={(event) => setStarts(event.currentTarget.value)} /></label><label>Ends<input type="datetime-local" value={ends()} onInput={(event) => setEnds(event.currentTarget.value)} /></label></div><label>Location<input type="text" value={location()} onInput={(event) => setLocation(event.currentTarget.value)} /></label><label>Description<textarea rows={5} value={description()} onInput={(event) => setDescription(event.currentTarget.value)} /></label></Show>
          <Show when={selected()?.source_refs.length}><p class="settings-hint">Based on: {selected()?.source_refs.map((item) => item.label || "Selected source").join(", ")}</p></Show>
          <div class="settings-actions"><button type="button" class="settings-button" disabled={busy()} onClick={() => setReview({ title: "Preview this saved draft", description: "This local preview does not contact the provider.", confirmLabel: "Close preview", reviewContent: <pre class="mail-calendar-review-payload">{JSON.stringify(actionForEditor(), null, 2)}</pre>, onConfirm: () => undefined })}>Preview draft</button><button type="button" class="btn primary" disabled={busy()} onClick={() => void save()}>{busy() ? "Saving…" : selected() ? "Save new revision" : "Save draft"}</button><Show when={selected()}>{(candidate) => <button type="button" class="btn danger" disabled={busy() || !candidate().candidate_digest || !!candidate().action_state} onClick={() => buildReview(candidate())}>Review exact provider action</button>}</Show></div>
          <Show when={revisionConflict() && selected()}>{(candidate) => <div class="settings-actions"><button type="button" class="settings-button" disabled={busy()} onClick={async () => { await refresh(props.agentId); const latest = candidates().find((item) => item.id === candidate().id); if (latest) beginOpen(latest); setRevisionConflict(false); }}>Discard edits and load latest</button><button type="button" class="settings-button" disabled={busy()} onClick={() => void save(true)}>Save edits as a separate draft</button></div>}</Show>
          <p class="settings-hint">Draft changes are saved only when you choose Save. A provider action uses the saved revision and its digest; editing it requires saving and reviewing again.</p>
        </div>
      </Show>
    </Show>
    <ConfirmModal config={review()} onClose={() => setReview(null)} />
  </section>;
}
