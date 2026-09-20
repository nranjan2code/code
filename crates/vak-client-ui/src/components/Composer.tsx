import { createEffect, createMemo, createSignal, onCleanup, onMount, For, Show, untrack } from "solid-js";
import {
  activeAgentId,
  activeId,
  armedGoal,
  health,
  isRunning,
  itemsOf,
  promptHistory,
  replyTarget,
  setReplyTarget,
  recordPrompt,
  setNotice,
  setArmedGoal,
  setDockTab,
  setShowShortcuts,
  switchModel,
  workspaceSwitching,
  agentForSession,
  agentOpening,
} from "../store";
import { loadHealth, openAgentChat, refreshSessions, sendPrompt, stopRun, switchWorkspace } from "../App";
import * as api from "../api";
import type { SkillInfo } from "../types";
import Icon from "./Icon";
import { IntentStrip } from "./IntentStrip";
import VoiceControl from "./VoiceControl";

interface Mention {
  start: number; // index of '@'
  query: string;
}

function detectMention(text: string, caret: number): Mention | null {
  const upto = text.slice(0, caret);
  const m = /(^|[\s(])@([\w./-]*)$/.exec(upto);
  if (!m) return null;
  return { start: caret - m[2].length - 1, query: m[2] };
}

interface SlashOption {
  id: string;
  name: string;
  kind: "command" | "skill";
  description: string;
}

const BUILTIN_SLASH_COMMANDS: { name: string; description: string }[] = [
  { name: "clear", description: "Clear current prompt draft and pending attachments" },
  { name: "btw", description: "Ask a side question without landing on main session chain" },
  { name: "compact", description: "Compact session context to free up context window tokens" },
  { name: "status", description: "Check current work without starting or steering a task" },
  { name: "diff", description: "Open diff inspector to review code changes" },
  { name: "terminal", description: "Open integrated shell terminal pane" },
  { name: "files", description: "Mention workspace files and attach code (@)" },
  { name: "help", description: "View keyboard shortcuts and command manual (?)" },
];

export default function Composer(props: { cwd: string }) {
  const [text, setText] = createSignal("");
  const [mention, setMention] = createSignal<Mention | null>(null);
  const [candidates, setCandidates] = createSignal<string[]>([]);
  const [picked, setPicked] = createSignal(0);
  const [files, setFiles] = createSignal<string[]>([]);
  const [skills, setSkills] = createSignal<SkillInfo[]>([]);
  const [commands, setCommands] = createSignal<api.CustomCommand[]>([]);
  const [slashPicked, setSlashPicked] = createSignal(0);
  // Image attachments: picked or pasted, sent as base64 vision blocks.
  const [pendingFiles, setPendingFiles] = createSignal<{ name: string; mime: string; data: string }[]>([]);
  const [composerError, setComposerError] = createSignal<string | null>(null);
  const [dragOver, setDragOver] = createSignal(false);
  const [historyIdx, setHistoryIdx] = createSignal(-1);
  let draftText = "";
  const [modelList, setModelList] = createSignal<string[]>([]);
  const [lookupError, setLookupError] = createSignal("");
  let ta!: HTMLTextAreaElement;
  let fileInput!: HTMLInputElement;
  const drafts = new Map<string, {text: string; files: { name: string; mime: string; data: string }[]}>();
  let draftOwner = "";
  createEffect(() => {
    const owner = `${props.cwd}:${activeId() ?? "vak"}`;
    untrack(() => {
      if (draftOwner) drafts.set(draftOwner, {text: text(), files: pendingFiles()});
      const draft = drafts.get(owner);
      setText(draft?.text ?? ""); setPendingFiles(draft?.files ?? []);
      setMention(null); setComposerError(null); setHistoryIdx(-1);
      draftOwner = owner;
      queueMicrotask(() => { if (ta) grow(); });
    });
  });

  const isTouchDevice = () => typeof window !== "undefined" && ("ontouchstart" in window || navigator.maxTouchPoints > 0);

  onMount(() => {
    const focus = () => ta?.focus();
    window.addEventListener("vak:focus-composer", focus);
    onCleanup(() => window.removeEventListener("vak:focus-composer", focus));
    const onEditPrompt = (ev: Event) => {
      const custom = ev as CustomEvent<{ text: string; mode?: "append" }>;
      if (custom.detail?.text) {
        setText((current) => custom.detail.mode === "append" && current.trim()
          ? `${current.trimEnd()}\n${custom.detail.text}`
          : custom.detail.text);
        queueMicrotask(() => {
          if (ta) {
            ta.focus();
            ta.setSelectionRange(ta.value.length, ta.value.length);
            grow();
          }
        });
      }
    };
    window.addEventListener("vak:edit-prompt", onEditPrompt);
    onCleanup(() => window.removeEventListener("vak:edit-prompt", onEditPrompt));
  });


  createEffect(() => {
    const p = health()?.provider;
    if (!p) return;
    api.discoverModels(p).then((r) => {
      setModelList(r.models);
      setLookupError("");
    }).catch((error) => {
      setModelList([]);
      setLookupError(`Model discovery unavailable: ${error instanceof Error ? error.message : String(error)}`);
    });
  });

  const MAX_FILE_BYTES = 5 * 1024 * 1024;

  const addFiles = (list: FileList | File[]) => {
    const owner = draftOwner;
    setComposerError(null);
    for (const file of Array.from(list)) {
      if (file.type.startsWith("image/")) {
        if (file.size > MAX_FILE_BYTES) {
          setComposerError(`${file.name} is too large (max 5 MB)`);
          continue;
        }
        const reader = new FileReader();
        reader.onload = () => {
          if (owner !== draftOwner) return;
          const url = String(reader.result ?? "");
          const base64 = url.includes(",") ? url.slice(url.indexOf(",") + 1) : "";
          if (base64) {
            setPendingFiles((cur) => [...cur, { name: file.name, mime: file.type, data: base64 }]);
          }
        };
        reader.readAsDataURL(file);
      } else {
        // Text / code file dropped: format and insert as code block
        if (file.size > 2 * 1024 * 1024) {
          setComposerError(`${file.name} is too large to attach (max 2 MB)`);
          continue;
        }
        const reader = new FileReader();
        reader.onload = () => {
          if (owner !== draftOwner) return;
          const content = String(reader.result ?? "");
          const ext = file.name.split(".").pop() ?? "";
          const codeBlock = `\n\`\`\`${ext}\n// ${file.name}\n${content}\n\`\`\`\n`;
          const cur = text();
          setText(cur ? `${cur}\n${codeBlock}` : codeBlock.trimStart());
          queueMicrotask(() => {
            if (ta) {
              ta.focus();
              ta.setSelectionRange(ta.value.length, ta.value.length);
              grow();
            }
          });
        };
        reader.readAsText(file);
      }
    }
  };


  const changeMode = async (mode: string) => {
    try {
      await api.setPermissionMode(mode, activeAgentId());
      await loadHealth();
    } catch (e) {
      setNotice({ kind: "error", text: `Could not change permission mode: ${e instanceof Error ? e.message : String(e)}` });
    }
  };

  // project file cache for @mentions (refresh when cwd changes)
  createEffect(() => {
    void props.cwd;
    setLookupError("");
    api
      .fsTree(600)
      .then((r) => setFiles(r.files))
      .catch((error) => setLookupError(`Workspace suggestions unavailable: ${error instanceof Error ? error.message : String(error)}`));
    api
      .listSkills()
      .then((r) => setSkills(r.skills))
      .catch((error) => setLookupError(`Skills unavailable: ${error instanceof Error ? error.message : String(error)}`));
    api
      .listCommands()
      .then((r) => setCommands(r.commands))
      .catch((error) => setLookupError(`Commands unavailable: ${error instanceof Error ? error.message : String(error)}`));
  });

  const slashMatches = (): SlashOption[] => {
    if (!ta || ta.selectionStart === 0 || !text().startsWith("/")) return [];
    const m = /^\/([\w-]*)$/.exec(text().slice(0, ta.selectionStart));
    if (!m) return [];
    const needle = m[1].toLowerCase();

    const cmds: SlashOption[] = BUILTIN_SLASH_COMMANDS
      .concat(commands())
      .filter((c) => !needle || c.name.toLowerCase().includes(needle) || c.description.toLowerCase().includes(needle))
      .map((c) => ({ id: `cmd-${c.name}`, name: c.name, kind: "command", description: c.description }));

    const sks: SlashOption[] = skills()
      .filter((s) => !needle || s.name.toLowerCase().includes(needle) || s.description.toLowerCase().includes(needle))
      .map((s) => ({ id: `skill-${s.name}`, name: s.name, kind: "skill", description: s.description || "Skill" }));

    return [...cmds, ...sks].slice(0, 10);
  };

  const applySlashOption = (option: SlashOption) => {
    if (option.kind === "command") {
      if (option.name === "clear") {
        setText("");
        setPendingFiles([]);
        setComposerError(null);
        queueMicrotask(grow);
        return;
      }
      if (option.name === "btw") {
        setText("/btw ");
        queueMicrotask(() => { ta.focus(); ta.setSelectionRange(5, 5); grow(); });
        return;
      }
      if (option.name === "files") {
        setText("");
        beginMention();
        return;
      }
      if (option.name === "diff") {
        setDockTab("diff");
        setText("");
        queueMicrotask(grow);
        return;
      }
      if (option.name === "terminal") {
        setDockTab("terminal");
        setText("");
        queueMicrotask(grow);
        return;
      }
      if (option.name === "help") {
        setShowShortcuts(true);
        setText("");
        queueMicrotask(grow);
        return;
      }
      if (option.name === "compact") {
        const next = "/compact ";
        setText(next);
        queueMicrotask(() => { ta.focus(); ta.setSelectionRange(next.length, next.length); grow(); });
        return;
      }
      const next = `/${option.name} `;
      setText(next);
      queueMicrotask(() => { ta.focus(); ta.setSelectionRange(next.length, next.length); grow(); });
    } else {
      const next = `use the ${option.name} skill `;
      setText(next);
      queueMicrotask(() => { ta.focus(); ta.setSelectionRange(next.length, next.length); grow(); });
    }
  };

  const beginSlash = () => {
    if (!text().startsWith("/")) {
      setText(`/${text().trimStart()}`);
    }
    queueMicrotask(() => {
      ta.focus();
      ta.setSelectionRange(text().length, text().length);
      grow();
    });
  };

  const matches = () => {
    const mn = mention();
    if (!mn) return [];
    const q = mn.query.toLowerCase();
    return files()
      .filter((f) => f.toLowerCase().includes(q))
      .slice(0, 8);
  };

  const refreshMention = () => {
    if (!ta) return;
    const mn = detectMention(text(), ta.selectionStart);
    if (!mn) {
      setMention(null);
      return;
    }
    setMention(mn);
    setPicked(0);
    setCandidates([]);
  };

  // keep candidate list reactive with the query
  createEffect(() => {
    const mn = mention();
    if (!mn) {
      setCandidates([]);
      return;
    }
    const q = mn.query.toLowerCase();
    setCandidates(files().filter((f) => f.toLowerCase().includes(q)).slice(0, 8));
  });

  const applyPick = (path: string) => {
    const mn = mention();
    if (!mn) return;
    const t = text();
    const next = `${t.slice(0, mn.start)}@${path} ${t.slice(ta.selectionStart)}`;
    setText(next);
    setMention(null);
    queueMicrotask(() => {
      const pos = mn.start + path.length + 2;
      ta.focus();
      ta.setSelectionRange(pos, pos);
      grow();
    });
  };

  const grow = () => {
    ta.style.height = "auto";
    ta.style.height = `${Math.min(ta.scrollHeight, 220)}px`;
  };

  const beginMention = () => {
    const spacer = text() && !text().endsWith(" ") ? " " : "";
    setText(`${text()}${spacer}@`);
    queueMicrotask(() => {
      ta.focus();
      ta.setSelectionRange(ta.value.length, ta.value.length);
      refreshMention();
      grow();
    });
  };

  const submit = async () => {
    if (agentOpening()) return;
    const t = text().trim();
    const files = pendingFiles();
    // No active task is fine: sendPrompt creates one.
    if (!t && files.length === 0) return;
    if (files.length > 0 && armedGoal()) {
      setComposerError("goal runs cannot carry images — disarm the goal or remove the attachments");
      return;
    }
    const target = replyTarget();
    if (t) recordPrompt(t);
    setHistoryIdx(-1);
    draftText = "";
    setText("");
    setMention(null);
    setPendingFiles([]);
    setComposerError(null);
    queueMicrotask(grow);
    // No need to pass or clear the goal here: sendPrompt consumes
    // `armedGoal` itself (store.ts), for whichever session it resolves.
    void sendPrompt(t, undefined, files.length ? files : undefined, target?.sessionId, target);
    setReplyTarget(null);
  };

  const submitVoice = (value: string) => {
    const prompt = value.trim();
    if (!prompt) return;
    recordPrompt(prompt);
  };

  const onKeyDown = (e: KeyboardEvent) => {
    const slashList = slashMatches();
    if (slashList.length) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setSlashPicked((p) => Math.min(p + 1, slashList.length - 1));
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setSlashPicked((p) => Math.max(p - 1, 0));
        return;
      }
      if (e.key === "Tab" || (e.key === "Enter" && !e.shiftKey)) {
        e.preventDefault();
        applySlashOption(slashList[Math.min(slashPicked(), slashList.length - 1)]);
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        setText("");
        return;
      }
    } else {
      setSlashPicked(0);
    }
    if (mention() && candidates().length) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setPicked((p) => Math.min(p + 1, candidates().length - 1));
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setPicked((p) => Math.max(p - 1, 0));
        return;
      }
      if (e.key === "Tab" || (e.key === "Enter" && !e.shiftKey)) {
        e.preventDefault();
        applyPick(candidates()[picked()]);
        return;
      }
      if (e.key === "Escape") {
        e.stopPropagation();
        setMention(null);
        return;
      }
    }

    // Prompt history navigation when input is empty or at start
    if (!slashList.length && !(mention() && candidates().length)) {
      if (e.key === "ArrowUp" && (ta.selectionStart === 0 || !text())) {
        const hist = promptHistory();
        if (hist.length > 0) {
          e.preventDefault();
          if (historyIdx() === -1) {
            draftText = text();
          }
          const nextIdx = Math.min(historyIdx() + 1, hist.length - 1);
          setHistoryIdx(nextIdx);
          setText(hist[nextIdx]);
          queueMicrotask(() => {
            ta.setSelectionRange(ta.value.length, ta.value.length);
            grow();
          });
          return;
        }
      }
      if (e.key === "ArrowDown" && historyIdx() >= 0) {
        e.preventDefault();
        const hist = promptHistory();
        const nextIdx = historyIdx() - 1;
        setHistoryIdx(nextIdx);
        if (nextIdx >= 0) {
          setText(hist[nextIdx]);
        } else {
          setText(draftText);
        }
        queueMicrotask(() => {
          ta.setSelectionRange(ta.value.length, ta.value.length);
          grow();
        });
        return;
      }
    }

    if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
      if (isTouchDevice() && !e.metaKey && !e.ctrlKey) {
        // Soft keyboards on touch devices: Return key creates a newline
        return;
      }
      e.preventDefault();
      submit();
    }
    // Escape without a menu falls through to the global stop handler.
  };

  return (
    <div class="composer-wrap">
      <Show when={replyTarget()}>
        {(target) => <div class="composer-target" role="status">
          <span>Replying to {target().label}</span>
          <button type="button" class="icon-button subtle" aria-label="Remove reply target" onClick={() => setReplyTarget(null)}>×</button>
        </div>}
      </Show>
      <Show when={slashMatches().length}>
        <div class="mention-menu slash-menu">
          <For each={slashMatches()}>
            {(opt, i) => (
              <button
                class="slash-item"
                classList={{ on: slashPicked() === i() }}
                onMouseEnter={() => setSlashPicked(i())}
                onClick={() => applySlashOption(opt)}
              >
                <span class="slash-badge" classList={{ cmd: opt.kind === "command", skill: opt.kind === "skill" }}>
                  {opt.kind === "command" ? "cmd" : "skill"}
                </span>
                <span class="slash-name">/{opt.name}</span>
                <span class="slash-desc">{opt.description}</span>
              </button>
            )}
          </For>
          <div class="mention-hint">commands & skills · Tab or Enter to use, Esc to dismiss</div>
        </div>
      </Show>
      <Show when={mention() && candidates().length}>
        <div class="mention-menu">
          <For each={candidates()}>
            {(f, i) => (
              <button
                class="mention-item"
                classList={{ on: picked() === i() }}
                onMouseEnter={() => setPicked(i())}
                onClick={() => applyPick(f)}
              >
                {f}
              </button>
            )}
          </For>
        </div>
      </Show>
      {/* Above the box, not inside it: this is a read-out about what you are
          about to send, and putting it inside the field would make it look
          like part of the message. */}
      <IntentStrip prompt={text()} sessionId={activeId()} disabled={isRunning(activeId())} />
      <Show when={lookupError()}>
        <div class="composer-lookup-error" role="status">{lookupError()} Suggestions are unavailable; you can still type and send a prompt.</div>
      </Show>
      <div
        class="composer-box"
        classList={{ running: isRunning(activeId()), "drag-over": dragOver() }}
        onDragOver={(e) => {
          e.preventDefault();
          setDragOver(true);
        }}
        onDragLeave={() => setDragOver(false)}
        onDrop={(e) => {
          e.preventDefault();
          setDragOver(false);
          if (e.dataTransfer?.files?.length) addFiles(e.dataTransfer.files);
        }}
      >
        <Show when={pendingFiles().length}>
          <div class="composer-attachments" aria-label="Attached images">
            <For each={pendingFiles()}>
              {(f, i) => (
                <span class="attachment-chip">
                  <img class="attachment-thumb" src={`data:${f.mime};base64,${f.data}`} alt="" />
                  <span class="attachment-name">{f.name}</span>
                  <button
                    class="attachment-remove"
                    title={`Remove ${f.name}`}
                    aria-label={`Remove ${f.name}`}
                    onClick={() => setPendingFiles((cur) => cur.filter((_, j) => j !== i()))}
                  >
                    ✕
                  </button>
                </span>
              )}
            </For>
          </div>
        </Show>
        <textarea
          ref={ta}
          rows={1}
          disabled={agentOpening()}
          aria-label={`Message ${agentForSession(activeId()).name}`}
          placeholder={`Ask ${agentForSession(activeId()).name} anything…`}
          value={text()}
          onInput={(event) => {
            setText(event.currentTarget.value);
            if (historyIdx() !== -1) setHistoryIdx(-1);
            refreshMention();
            grow();
          }}
          onKeyUp={(event) => { if (["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) refreshMention(); }}
          onClick={refreshMention}
          onKeyDown={onKeyDown}
          onPaste={(e) => {
            const files = Array.from(e.clipboardData?.files ?? []);
            if (files.length) {
              e.preventDefault();
              addFiles(files);
            }
          }}
        />
        <div class="composer-toolbar">
          <div class="composer-lead">
            <details class="composer-more">
              <summary class="composer-context" aria-label="More ways to work"><Icon name="more" size={14} /><span>More</span></summary>
              <div class="composer-more-menu">
                <button type="button" onClick={() => void switchWorkspace()}><Icon name="folder" size={14} /><span>{workspaceSwitching() ? "Opening…" : `Target: ${props.cwd.split("/").pop() || "root"}`}</span></button>
                <button type="button" onClick={beginMention}><span class="composer-at">@</span><span>Mention a file</span></button>
                <button type="button" onClick={beginSlash}><span class="composer-at">/</span><span>Use a skill or command</span></button>
                <select
                  class="composer-mode composer-model-select"
                  aria-label="Model"
                  value={health()?.model ?? ""}
                  onChange={(e) => void switchModel(e.currentTarget.value)}
                  disabled={modelList().length === 0}
                >
                  <Show when={modelList().length === 0}><option>Model unavailable</option></Show>
                  <For each={modelList()}>
                    {(m) => <option value={m}>{m}</option>}
                  </For>
                </select>
                <select class="composer-mode" aria-label="Permission mode" value={health()?.permission_mode ?? ""} onChange={(e) => void changeMode(e.currentTarget.value)}>
                  <option value="ReadOnly">Read only</option>
                  <option value="WorkspaceWrite">Workspace write</option>
                  <option value="FullAccess">Full access</option>
                </select>
              </div>
            </details>
            <VoiceControl sessionId={activeId() ?? undefined} ensureSession={() => openAgentChat(activeAgentId())} onFinal={submitVoice} />
            <input
              ref={fileInput}
              type="file"
              accept="image/*,.txt,.rs,.ts,.js,.json,.py,.md,.toml,.yaml,.yml,.css,.html,.sh"
              multiple
              style="display:none"
              onChange={(e) => {
                if (e.currentTarget.files?.length) addFiles(e.currentTarget.files);
                e.currentTarget.value = "";
              }}
            />
            <button
              class="composer-context composer-attach"
              title="Attach files (or paste / drop them here)"
              aria-label="Attach files"
              onClick={() => fileInput.click()}
            >
              <Icon name="add" size={13} />
              <span>Attach</span>
            </button>
          </div>

          <Show when={armedGoal()}>
            <div class="goal-chip" title="Goal mode armed (docs/design/27 Phase H) — completion will be audited against the criteria">
              <Icon name="spark" size={13} />
              <span>{armedGoal()!.objective.slice(0, 60)}</span>
              {" · "}
              {armedGoal()!.criteria.length} criteria
              <button
                class="goal-disarm"
                title="Disarm goal"
                onClick={() => setArmedGoal(null)}
              >
                ✕
              </button>
            </div>
          </Show>
          <div class="composer-actions">
            <Show
              when={isRunning(activeId())}
              fallback={
                <button
                  class="send-button"
                  title="Send prompt (Enter)"
                  aria-label="Send prompt"
                  disabled={!text().trim() && pendingFiles().length === 0}
                  onClick={submit}
                >
                  <Icon name="send" size={15} />
                </button>
              }
            >
              <button
                class="send-button stop"
                title="Stop running task (Esc)"
                aria-label="Stop running task"
                onClick={stopRun}
              >
                <Icon name="stop" size={15} />
              </button>
            </Show>
          </div>
        </div>
      </div>
      <Show when={composerError()}>
        <div class="composer-error" role="alert">{composerError()}</div>
      </Show>
      <div class="composer-note">
        Vak can make mistakes. Check important information and review consequential actions.
      </div>
    </div>
  );
}
