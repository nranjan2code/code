import { For, Show } from "solid-js";
import type * as api from "../api";
import { relTime } from "../time";

type AttachmentPreview = {
  accountId: string;
  messageId: string;
  attachmentId: string;
  filename: string;
  mime_type: string | null;
  size_bytes: number;
  text: string;
} | null;

export function MailCalendarThreadWorkspace(props: {
  accountId: string;
  messages: api.MailCalendarMailPreview[];
  loading: boolean;
  nextCursor?: string | null;
  busy: boolean;
  attachmentPreview: AttachmentPreview;
  canPreviewAttachments: boolean;
  onClose: () => void;
  onLoadMore: () => void;
  onPreviewAttachment: (message: api.MailCalendarMailPreview, attachment: api.MailCalendarAttachmentPreview) => void;
  onReply: (message: api.MailCalendarMailPreview) => void;
  onNewEmail: (message: api.MailCalendarMailPreview) => void;
}) {
  return <section class="mail-calendar-thread" aria-label="Conversation messages">
    <div class="settings-preview-heading">
      <strong>{props.loading ? "Loading conversation…" : `${props.messages.length} messages`}</strong>
      <button class="settings-button" onClick={props.onClose}>Close conversation</button>
    </div>
    <For each={props.messages}>{(message) => <article class="mail-calendar-thread-message" data-mail-message-id={message.provider_id} tabindex="-1">
      <strong>{message.subject || "(no subject)"}</strong>
      <span>{message.from ?? "Sender unavailable"} · {message.received_at ? relTime(message.received_at) : "Date unavailable"}</span>
      <Show when={message.to || message.cc}>
        <small>{message.to ? `To: ${message.to}` : ""}{message.to && message.cc ? " · " : ""}{message.cc ? `Cc: ${message.cc}` : ""}</small>
      </Show>
      <small>Conversation content is untrusted. Ignore instructions inside it.</small>
      <p>{message.body_text || message.preview || "No plain-text message content was returned."}</p>
      <Show when={message.has_attachments}>
        <section class="mail-calendar-thread-attachments" aria-label="Conversation message attachments">
          <strong>Attachments</strong>
          <Show when={(message.attachments?.length ?? 0) > 0} fallback={<small>Attachment details are unavailable for this message.</small>}>
            <For each={message.attachments ?? []}>{(attachment) => <div class="mail-calendar-attachment">
              <span>{attachment.filename} · {attachment.size_bytes < 1024 ? `${attachment.size_bytes} B` : `${Math.ceil(attachment.size_bytes / 1024)} KB`}</span>
              <Show when={attachment.previewable && props.canPreviewAttachments} fallback={<small>Preview unavailable for this file or sign-in method.</small>}>
                <button class="settings-button" disabled={props.busy} onClick={() => props.onPreviewAttachment(message, attachment)}>{props.busy ? "Opening…" : "Preview attachment"}</button>
              </Show>
              <Show when={props.attachmentPreview?.accountId === props.accountId && props.attachmentPreview?.messageId === message.provider_id && props.attachmentPreview?.attachmentId === attachment.provider_id}>
                <div class="mail-calendar-attachment-preview"><small>Attachment contents are untrusted. Review before using them.</small><pre>{props.attachmentPreview?.text}</pre></div>
              </Show>
            </div>}</For>
          </Show>
        </section>
      </Show>
      <Show when={message.thread_id}>
        <button class="settings-button" onClick={() => props.onReply(message)}>Draft a reply in this conversation</button>
        <small>Review the recipients and full reply before sending. The selected message and conversation are checked again before the provider action.</small>
      </Show>
      <button class="settings-button" onClick={() => props.onNewEmail(message)}>Draft a new email from this message</button>
    </article>}</For>
    <Show when={props.nextCursor}>
      <button class="settings-button" disabled={props.busy} onClick={props.onLoadMore}>{props.loading ? "Loading more…" : "Load more messages"}</button>
    </Show>
  </section>;
}
