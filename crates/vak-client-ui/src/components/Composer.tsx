import { createEffect, createMemo, createSignal, onCleanup, onMount, For, Show } from "solid-js";
import type { JSX } from "solid-js";
import {
  activeId,
  armedGoal,
  density,
  health,
  isRunning,
  itemsOf,
  promptHistory,
  recordPrompt,
  setArmedGoal,
  setDensity,
  switchModel,
  usageOf,
  workspaceSwitching,
} from "../store";
import type { Density } from "../store";
import { loadHealth, sendPrompt, stopRun, switchWorkspace } from "../App";
import * as api from "../api";
import type { SkillInfo } from "../types";
import Icon from "./Icon";
import { IntentStrip } from "./IntentStrip";

/** Context-window gauge; sits with the run controls in the composer. */
function Ring(props: { pct: number; label: string }): JSX.Element {
  const r = 8;
  const c = 2 * Math.PI * r;
  const clamped = () => Math.max(0, Math.min(1, props.pct));
  // Fixed status colors (never the accent) so the ring's meaning — normal,
  // approaching the limit, over it — reads the same in every theme,
  // instead of pinning to warm terracotta regardless of the active accent.
  const color = () => (clamped() > 0.85 ? "var(--red)" : clamped() > 0.6 ? "var(--yellow)" : "var(--accent)");
  return (
    <div class="ring" title={props.label}>
      <svg width="20" height="20" viewBox="0 0 20 20">
        <circle cx="10" cy="10" r={r} fill="none" stroke="var(--border)" stroke-width="2.5" />
        <circle
          cx="10"
          cy="10"
          r={r}
          fill="none"
          stroke={color()}
          stroke-width="2.5"
          stroke-linecap="round"
          stroke-dasharray={`${clamped() * c} ${c}`}
          transform="rotate(-90 10 10)"
        />
      </svg>
    </div>
  );
}

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

/** Slash palette: `/name` as the very start of the message picks a skill. */
function detectSkillQuery(text: string, caret: number): string | null {
  if (caret === 0 || !text.startsWith("/")) return null;
  const upto = text.slice(0, caret);
  const m = /^\/([\w-]*)$/.exec(upto);
  return m ? m[1] : null;
}

export default function Composer(props: { cwd: string }) {
  const [text, setText] = createSignal("");
  const [mention, setMention] = createSignal<Mention | null>(null);
  const [candidates, setCandidates] = createSignal<string[]>([]);
  const [picked, setPicked] = createSignal(0);
  const [files, setFiles] = createSignal<string[]>([]);
  const [skills, setSkills] = createSignal<SkillInfo[]>([]);
  const [commands, setCommands] = createSignal<api.CustomCommand[]>([]);
  const [commandPicked, setCommandPicked] = createSignal(0);
  const [skillPicked, setSkillPicked] = createSignal(0);
  // Image attachments: picked or pasted, sent as base64 vision blocks.
  const [pendingFiles, setPendingFiles] = createSignal<{ name: string; mime: string; data: string }[]>([]);
  const [composerError, setComposerError] = createSignal<string | null>(null);
  const [dragOver, setDragOver] = createSignal(false);
  const [historyIdx, setHistoryIdx] = createSignal(-1);
  let draftText = "";
  const [modelList, setModelList] = createSignal<string[]>([]);
  let ta!: HTMLTextAreaElement;
  let fileInput!: HTMLInputElement;

  const isTouchDevice = () => typeof window !== "undefined" && ("ontouchstart" in window || navigator.maxTouchPoints > 0);

  onMount(() => {
    const onEditPrompt = (ev: Event) => {
      const custom = ev as CustomEvent<{ text: string }>;
      if (custom.detail?.text) {
        setText(custom.detail.text);
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
    api.discoverModels(p).then((r) => setModelList(r.models)).catch(() => {});
  });

  const MAX_FILE_BYTES = 5 * 1024 * 1024;

  const addFiles = (list: FileList | File[]) => {
    setComposerError(null);
    for (const file of Array.from(list)) {
      if (file.type.startsWith("image/")) {
        if (file.size > MAX_FILE_BYTES) {
          setComposerError(`${file.name} is too large (max 5 MB)`);
          continue;
        }
        const reader = new FileReader();
        reader.onload = () => {
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


  const usage = createMemo(() => usageOf(activeId()));
  const ctxPct = createMemo(() => {
    const w = health()?.context_window ?? 0;
    return w > 0 ? (usage().input_tokens ?? 0) / w : 0;
  });
  const inTok = createMemo(() => (usage().input_tokens ?? 0).toLocaleString());
  const outTok = createMemo(() => (usage().output_tokens ?? 0).toLocaleString());

  const changeMode = async (mode: string) => {
    try {
      await api.setPermissionMode(mode);
      await loadHealth();
    } catch (e) {
      console.error(e);
    }
  };

  // project file cache for @mentions (refresh when cwd changes)
  createEffect(() => {
    void props.cwd;
    api
      .fsTree(600)
      .then((r) => setFiles(r.files))
      .catch(() => setFiles([]));
    api
      .listSkills()
      .then((r) => setSkills(r.skills))
      .catch(() => setSkills([]));
    api
      .listCommands()
      .then((r) => setCommands(r.commands))
      .catch(() => setCommands([]));
  });

  const skillMatches = () => {
    // The skill menu renders above the textarea, so this runs once before
    // `ta` is assigned; without the guard that first pass throws and takes
    // down whatever triggered the render.
    if (!ta) return [];
    if (commandMatches().length) return [];
    const q = detectSkillQuery(text(), ta.selectionStart);
    if (q === null) return [];
    const needle = q.toLowerCase();
    return skills()
      .filter(
        (s) =>
          s.name.toLowerCase().includes(needle) ||
          s.description.toLowerCase().includes(needle),
      )
      .slice(0, 8);
  };

  const commandMatches = () => {
    if (!ta || ta.selectionStart === 0 || !text().startsWith("/")) return [];
    const m = /^\/([\w-]*)$/.exec(text().slice(0, ta.selectionStart));
    if (!m) return [];
    const needle = m[1].toLowerCase();
    return commands().filter((c) => c.name.includes(needle) || c.description.toLowerCase().includes(needle)).slice(0, 8);
  };

  const applyCommand = (command: api.CustomCommand) => {
    const next = `/${command.name} `;
    setText(next);
    queueMicrotask(() => { ta.focus(); ta.setSelectionRange(next.length, next.length); grow(); });
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

  const applySkill = (skill: SkillInfo) => {
    const next = `use the ${skill.name} skill `;
    setText(next);
    queueMicrotask(() => {
      ta.focus();
      ta.setSelectionRange(next.length, next.length);
      grow();
    });
  };

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

  const submit = () => {
    const t = text().trim();
    const files = pendingFiles();
    // No active task is fine: sendPrompt creates one.
    if (!t && files.length === 0) return;
    if (files.length > 0 && armedGoal()) {
      setComposerError("goal runs cannot carry images — disarm the goal or remove the attachments");
      return;
    }
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
    void sendPrompt(t, undefined, files.length ? files : undefined);
  };

  const onKeyDown = (e: KeyboardEvent) => {
    const commandMenu = commandMatches();
    if (commandMenu.length) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setCommandPicked((p) => Math.min(p + 1, commandMenu.length - 1));
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setCommandPicked((p) => Math.max(p - 1, 0));
        return;
      }
      if (e.key === "Tab" || (e.key === "Enter" && !e.shiftKey)) {
        e.preventDefault(); applyCommand(commandMenu[Math.min(commandPicked(), commandMenu.length - 1)]); return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        setText("");
        return;
      }
    } else {
      setCommandPicked(0);
    }
    const skillMenu = skillMatches();
    if (skillMenu.length) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setSkillPicked((p) => Math.min(p + 1, skillMenu.length - 1));
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setSkillPicked((p) => Math.max(p - 1, 0));
        return;
      }
      if (e.key === "Tab" || (e.key === "Enter" && !e.shiftKey)) {
        e.preventDefault();
        applySkill(skillMenu[Math.min(skillPicked(), skillMenu.length - 1)]);
        return;
      }
      if (e.key === "Escape") {
        e.stopPropagation();
        setText(text().slice(1));
        queueMicrotask(() => ta.setSelectionRange(0, 0));
        return;
      }
    } else {
      setSkillPicked(0);
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
    if (!commandMenu.length && !skillMenu.length && !(mention() && candidates().length)) {
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
      <Show when={commandMatches().length}>
        <div class="mention-menu skill-menu">
          <For each={commandMatches()}>{(command, i) => <button class="mention-item" classList={{ on: commandPicked() === i() }} onMouseEnter={() => setCommandPicked(i())} onClick={() => applyCommand(command)}><span class="skill-name">/{command.name}</span><span class="skill-desc">{command.description}</span></button>}</For>
          <div class="mention-hint">commands · Tab or Enter to insert</div>
        </div>
      </Show>
      <Show when={skillMatches().length}>
        <div class="mention-menu skill-menu">
          <For each={skillMatches()}>
            {(s, i) => (
              <button
                class="mention-item"
                classList={{ on: skillPicked() === i() }}
                onMouseEnter={() => setSkillPicked(i())}
                onClick={() => applySkill(s)}
              >
                <span class="skill-name">/{s.name}</span>
                <span class="skill-desc">{s.description || "no description"}</span>
              </button>
            )}
          </For>
          <div class="mention-hint">skills · Enter to use, Esc to dismiss</div>
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
      <IntentStrip prompt={text()} disabled={isRunning(activeId())} />
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
          placeholder={activeId() && isRunning(activeId()) ? "Add direction while Vak is working…" : "Ask Vak to build, fix, or explain…"}
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
            <button
              class="composer-project"
              title={`${props.cwd} — click to switch workspace`}
              onClick={() => void switchWorkspace()}
            >
              <Icon name="folder" size={14} />
              <span>{workspaceSwitching() ? "Opening…" : props.cwd.split("/").pop()}</span>
              <Icon name="chevron" size={12} />
            </button>
            <select
              class="composer-mode"
              value={health()?.permission_mode ?? ""}
              onChange={(e) => void changeMode(e.currentTarget.value)}
              title="Permission mode — applies to new tool calls immediately"
            >
              <option value="ReadOnly">Read only</option>
              <option value="WorkspaceWrite">Workspace write</option>
              <option value="FullAccess">Full access</option>
            </select>
            <Show when={modelList().length > 0}>
              <select
                class="composer-mode composer-model-select"
                value={health()?.model ?? ""}
                onChange={(e) => void switchModel(e.currentTarget.value)}
                title={`Active model: ${health()?.model ?? ""} — click to switch`}
              >
                <For each={modelList()}>
                  {(m) => <option value={m}>{m}</option>}
                </For>
              </select>
            </Show>
            <button class="composer-context" title="Mention file (@)" onClick={beginMention}>
              <span class="composer-at">@</span>
              <span>files</span>
            </button>
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
            <select
              class="composer-density"
              value={density()}
              onChange={(e) => setDensity(e.currentTarget.value as Density)}
              aria-label="Transcript detail"
              title="Transcript detail"
            >
              <option value="outcome">outcome</option>
              <option value="balanced">balanced</option>
              <option value="audit">audit</option>
            </select>
            <span class="composer-tokens" title={`Input ${inTok()} tokens · output ${outTok()} tokens`}>
              {inTok()} in · {outTok()} out
            </span>
            <Ring
              pct={ctxPct()}
              label={`context: ${(ctxPct() * 100).toFixed(0)}% of ${((health()?.context_window ?? 0) / 1000).toFixed(0)}k`}
            />
            <Show when={isRunning(activeId())}>
              <button class="composer-stop" title="Stop (Esc)" aria-label="Stop running task" onClick={stopRun}><Icon name="stop" size={15} /><span>Stop</span></button>
            </Show>
            <button class="send-button" title="Send (Enter)" aria-label="Send prompt" disabled={!text().trim() && pendingFiles().length === 0} onClick={submit}>
              <Icon name="send" size={16} />
            </button>
          </div>
        </div>
      </div>
      <Show when={composerError()}>
        <div class="composer-error" role="alert">{composerError()}</div>
      </Show>
      <div class="composer-note">Vak can make mistakes. Review changes before you keep them.</div>
    </div>
  );
}
