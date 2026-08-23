import { createEffect, createMemo, createSignal, For, Show } from "solid-js";
import type { JSX } from "solid-js";
import { activeId, density, health, isRunning, itemsOf, setDensity, usageOf } from "../store";
import type { Density } from "../store";
import { loadHealth, sendPrompt, stopRun, switchProject } from "../App";
import * as api from "../api";
import type { SkillInfo } from "../types";
import Icon from "./Icon";

/** Context-window gauge; sits with the run controls in the composer. */
function Ring(props: { pct: number; label: string }): JSX.Element {
  const r = 9;
  const c = 2 * Math.PI * r;
  const clamped = () => Math.max(0, Math.min(1, props.pct));
  const color = () => (clamped() > 0.85 ? "#d86f72" : clamped() > 0.6 ? "#d4a85d" : "#df795f");
  return (
    <div class="ring" title={props.label}>
      <svg width="24" height="24" viewBox="0 0 24 24">
        <circle cx="12" cy="12" r={r} fill="none" stroke="#34342f" stroke-width="3" />
        <circle
          cx="12"
          cy="12"
          r={r}
          fill="none"
          stroke={color()}
          stroke-width="3"
          stroke-linecap="round"
          stroke-dasharray={`${clamped() * c} ${c}`}
          transform="rotate(-90 12 12)"
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
  const [skillPicked, setSkillPicked] = createSignal(0);
  let ta!: HTMLTextAreaElement;

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
  });

  const skillMatches = () => {
    // The skill menu renders above the textarea, so this runs once before
    // `ta` is assigned; without the guard that first pass throws and takes
    // down whatever triggered the render.
    if (!ta) return [];
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
    if (!t || !activeId()) return;
    setText("");
    setMention(null);
    queueMicrotask(grow);
    void sendPrompt(t);
  };

  const onKeyDown = (e: KeyboardEvent) => {
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
    if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      submit();
    }
    // Escape without a menu falls through to the global stop handler.
  };

  return (
    <div class="composer-wrap">
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
      <div class="composer-box" classList={{ running: isRunning(activeId()) }}>
        <textarea
          ref={ta}
          rows={1}
          placeholder={activeId() ? (isRunning(activeId()) ? "Add direction while vakcoder is working…" : "Ask vakcoder to build, fix, or explain…") : "Start or select a task…"}
          value={text()}
          onInput={(event) => { setText(event.currentTarget.value); refreshMention(); grow(); }}
          onKeyUp={(event) => { if (["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) refreshMention(); }}
          onClick={refreshMention}
          onKeyDown={onKeyDown}
        />
        <div class="composer-toolbar">
          <div class="composer-lead">
            <button
              class="composer-project"
              title={`${props.cwd} — click to switch project`}
              onClick={() => void switchProject()}
            >
              <Icon name="folder" size={14} />
              <span>{props.cwd.split("/").pop()}</span>
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
            <button class="composer-context" title="Add file context (@)" onClick={beginMention}>
              <span class="composer-hint">@ to add files</span>
            </button>
          </div>
          <div class="composer-actions">
            <select
              class="composer-density"
              value={density()}
              onChange={(e) => setDensity(e.currentTarget.value as Density)}
              title="Transcript detail"
            >
              <option value="summary">summary</option>
              <option value="normal">normal</option>
              <option value="verbose">verbose</option>
            </select>
            <span class="composer-tokens" title={`in ${inTok()} / out ${outTok()}`}>
              ↑{inTok()} ↓{outTok()}
            </span>
            <Ring
              pct={ctxPct()}
              label={`context: ${(ctxPct() * 100).toFixed(0)}% of ${((health()?.context_window ?? 0) / 1000).toFixed(0)}k`}
            />
            <Show when={isRunning(activeId())}>
              <button class="composer-stop" title="Stop (Esc)" aria-label="Stop running task" onClick={stopRun}><Icon name="stop" size={15} /><span>Stop</span></button>
            </Show>
            <button class="send-button" title="Send (Enter)" aria-label="Send prompt" disabled={!activeId() || !text().trim()} onClick={submit}>
              <Icon name="send" size={16} />
            </button>
          </div>
        </div>
      </div>
      <div class="composer-note">vakcoder can make mistakes. Review changes before you keep them.</div>
    </div>
  );
}
