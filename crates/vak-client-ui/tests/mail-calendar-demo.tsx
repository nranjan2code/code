import * as api from "../src/api";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { setSyntheticMailCalendarEnabled, syntheticMailCalendarEnabled } from "../src/mailCalendarDemo";
import { SyntheticMailCalendarDemoButton, SyntheticMailCalendarDemoControl } from "../src/components/SyntheticMailCalendarDemoControl";
import { MailCalendarThreadWorkspace } from "../src/components/MailCalendarThreadWorkspace";

const routes: string[] = [];
window.fetch = async (input) => {
  routes.push(String(input));
  return new Response("{}", { headers: { "Content-Type": "application/json" } });
};
const check = (condition: unknown, label: string) => { if (!condition) throw new Error(label); return label; };
const expectRefused = async (action: () => Promise<unknown>) => {
  try { await action(); return false; }
  catch (error) { return error instanceof api.ApiError && error.status === 403 && error.kind === "synthetic_demo_read_only"; }
};
setSyntheticMailCalendarEnabled(false);
const [demoEnabled, setDemoEnabled] = createSignal(false);
render(() => <div class="settings"><SyntheticMailCalendarDemoControl checked={demoEnabled()} onChange={(enabled) => { setSyntheticMailCalendarEnabled(enabled); setDemoEnabled(enabled); }} /></div>, document.getElementById("root")!);
const [demoButtonEnabled, setDemoButtonEnabled] = createSignal(false);
const buttonRoot = document.createElement("div");
document.body.append(buttonRoot);
render(() => <SyntheticMailCalendarDemoButton checked={demoButtonEnabled()} onChange={(enabled) => { setSyntheticMailCalendarEnabled(enabled); setDemoButtonEnabled(enabled); }} />, buttonRoot);
(window as any).runChecks = async () => {
  window.localStorage.removeItem("vak.mail-calendar.synthetic-drafts.v1.fixture-agent");
  const toggle = document.querySelector<HTMLInputElement>("[data-testid='synthetic-mail-calendar-toggle']");
  toggle?.click();
  await Promise.resolve();
  const activatedFromSettings = demoEnabled() && !!toggle?.checked;
  const demoButton = document.querySelector<HTMLButtonElement>("[data-testid='synthetic-mail-calendar-button']");
  demoButton?.click();
  const activatedFromToday = demoButtonEnabled() && syntheticMailCalendarEnabled();
  const safeModeExplanationVisible = document.body.textContent?.includes("Nothing is read from or written to a provider");
  if (!activatedFromSettings) throw new Error("Settings demo switch did not enable synthetic mode");
  const inventory = await api.listMailCalendarAccounts("fixture-agent");
  const folders = await api.listMailCalendarFolders("fixture-agent", "demo-microsoft-1");
  const nestedFolderMail = await api.previewMailCalendarMail("fixture-agent", "demo-microsoft-1", 8, undefined, "archive-projects");
  const mail = await api.previewMailCalendarMail("fixture-agent", "demo-google-1", 8);
  const thread = await api.previewMailCalendarThread("fixture-agent", "demo-google-1", "demo-thread-fixture");
  const [threadAttachmentPreview, setThreadAttachmentPreview] = createSignal<{
    accountId: string; messageId: string; attachmentId: string; filename: string; mime_type: string | null; size_bytes: number; text: string;
  } | null>(null);
  let attachmentActionStatus = "not requested";
  render(() => <MailCalendarThreadWorkspace
    accountId="demo-google-1"
    messages={thread.messages}
    loading={false}
    busy={false}
    attachmentPreview={threadAttachmentPreview()}
    canPreviewAttachments={true}
    onClose={() => undefined}
    onLoadMore={() => undefined}
    onPreviewAttachment={(message, attachment) => void api.previewMailCalendarAttachment("fixture-agent", "demo-google-1", message.provider_id, attachment.provider_id).then((result) => {
      attachmentActionStatus = `loaded ${result.text}`;
      setThreadAttachmentPreview({ accountId: "demo-google-1", messageId: message.provider_id, attachmentId: attachment.provider_id, ...result });
    }).catch((error: unknown) => { attachmentActionStatus = `failed ${error instanceof Error ? error.message : String(error)}`; })}
    onReply={() => undefined}
    onNewEmail={() => undefined}
  />, document.getElementById("thread-root")!);
  const calendar = await api.previewMailCalendarEvents("fixture-agent", "demo-google-1", new Date().toISOString(), new Date(Date.now() + 86_400_000).toISOString());
  const saved = await api.saveMailCalendarCandidate("fixture-agent", { account_id: "local-draft", action: { kind: "create_event", draft: { title: "Practice", description: "", location: null, starts_at: new Date().toISOString(), ends_at: new Date(Date.now() + 3_600_000).toISOString(), time_zone: "UTC", all_day: false, attendee_addresses: [], recurrence: null, occurrence_id: null } } });
  const listed = await api.listMailCalendarCandidates("fixture-agent");
  [...document.querySelectorAll<HTMLButtonElement>("#thread-root button")]
    .find((button) => button.textContent?.includes("Preview attachment"))?.click();
  const attachmentPreviewWorks = await new Promise<boolean>((resolve) => {
    const deadline = Date.now() + 5000;
    const poll = () => {
      if (document.querySelector("#thread-root .mail-calendar-attachment-preview")?.textContent?.includes("Synthetic attachment preview")) resolve(true);
      else if (Date.now() >= deadline) resolve(false);
      else setTimeout(poll, 25);
    };
    poll();
  });
  const deniedConnect = await expectRefused(() => api.beginMailCalendarOAuth("fixture-agent", "google", ["mail_read"]));
  const deniedEffect = await expectRefused(() => api.createMailCalendarEventCandidate("fixture-agent", saved.candidate.id, saved.candidate.revision, "demo-digest"));
  toggle?.click();
  const switchTurnsOff = !demoEnabled() && !toggle?.checked;
  const passed = [
    check(activatedFromSettings && safeModeExplanationVisible, "The Settings checkbox activates the clearly labelled safe demo mode"),
    check(activatedFromToday, "The Today view button activates synthetic mode without opening Settings"),
    check(inventory.accounts.length === 9 && new Set(inventory.accounts.map((item) => item.provider)).size === 3, "Nine labelled demo accounts cover Google, Microsoft and Apple"),
    check(inventory.accounts.every((item) => item.identity_masked?.endsWith("@example.test") && item.capabilities.every((capability) => capability !== "mail_send" && capability !== "calendar_write")), "Accounts use example.test identities and read-only capabilities"),
    check(folders.folders.some((folder) => folder.provider_id === "archive-projects" && folder.name === "Archive / Projects") && nestedFolderMail.messages.length === 8, "Synthetic mode lets the owner preview a nested Microsoft folder"),
    check(mail.messages.length === 8 && mail.messages.every((item) => item.body_text?.toLowerCase().includes("synthetic")), "Mail previews are generated synthetic samples"),
    check(thread.messages.length === 1 && thread.messages[0].has_attachments && thread.messages[0].attachments?.[0]?.filename === "sample-notes.txt", "Conversation preview exposes its generated attachment card"),
    check(attachmentPreviewWorks && attachmentActionStatus.startsWith("loaded "), "The real conversation workspace previews the generated attachment locally"),
    check(calendar.events.length > 0 && calendar.events.every((item) => item.description?.includes("Synthetic")), "Calendar preview is generated synthetic sample data"),
    check(listed.candidates.some((item) => item.id === saved.candidate.id && item.action.kind === "create_event"), "Drafts save and reopen in browser-local demo storage"),
    check(deniedConnect, "OAuth connection attempts are refused in synthetic mode"),
    check(deniedEffect, "Provider effects are refused in synthetic mode"),
    check(routes.length === 0, "No mail/calendar API request escapes to the server in demo mode"),
    check(switchTurnsOff, "The same Settings checkbox restores real-account mode"),
  ];
  const report = document.createElement("pre");
  report.id = "fixture-report";
  report.textContent = `${passed.length} checks passed\n${passed.join("\n")}`;
  document.body.append(report);
  return passed;
};
if (new URLSearchParams(location.search).has("run")) {
  (window as any).runChecks().catch((error: unknown) => {
    const report = document.createElement("pre"); report.id = "fixture-report"; report.setAttribute("role", "alert");
    report.textContent = `Failed: ${error instanceof Error ? error.message : String(error)}`; document.body.append(report);
  });
}
