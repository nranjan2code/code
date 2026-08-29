/// Plain-language controls shared by the admin screens.
///
/// Every control here is a presentation layer over a wire shape that was
/// previously typed by hand — a glob list, a cron expression, a permission
/// matcher. None of them change what is sent; they only stop the console
/// from asking an everyday operator to author syntax. Each one keeps an
/// "advanced" escape hatch so anything the picker cannot express is still
/// editable, and anything already configured that the picker does not
/// recognise is preserved and shown rather than silently dropped.

import { For, Show, createEffect, createMemo, createRenderEffect, createSignal } from "solid-js";

// ---- Access picker ---------------------------------------------------------

export interface AccessOption {
  /// The value written to the wire list (a glob, a name, an identity).
  value: string;
  /// What the operator reads.
  label: string;
  /// Optional one-line clarification under the label.
  hint?: string;
}

export type AccessMode = "inherit" | "limit" | "none";

/// `allow` is tri-state on the wire: `null` inherits, `[]` blocks everything,
/// a non-empty list limits to those entries. Read that back into the three
/// words an operator actually thinks in.
export function accessMode(allow: string[] | null): AccessMode {
  if (allow === null) return "inherit";
  return allow.length === 0 ? "none" : "limit";
}

/// A three-way choice plus a checklist, replacing two free-text glob fields.
/// `options` are the things that actually exist right now (discovered MCP
/// servers, registered skills, the built-in tools); anything already in the
/// list that is not among them stays as a removable custom chip.
export function AccessPicker(props: {
  title: string;
  /// What "inherit" means for this particular capability, in one line.
  inheritLabel: string;
  limitLabel: string;
  noneLabel: string;
  /// Noun for the things being picked, e.g. "server", "skill".
  noun: string;
  options: AccessOption[];
  loading?: boolean;
  allow: string[] | null;
  deny: string[];
  onChange: (next: { allow: string[] | null; deny: string[] }) => void;
  /// Shown under the heading. Say what the restriction does, not how it is stored.
  help?: string;
  /// Placeholder for the advanced pattern field.
  patternHint?: string;
}) {
  const [pattern, setPattern] = createSignal("");
  const [showAdvanced, setShowAdvanced] = createSignal(false);
  // "Limit to what I pick" and "block all" are the same `[]` on the wire, so
  // the wire alone cannot tell them apart the moment the operator switches to
  // "limit" and has picked nothing yet. Remember the choice locally and let
  // it win, or the control would snap straight back to "block all".
  const [picked, setPicked] = createSignal<AccessMode | null>(null);
  const mode = () => picked() ?? accessMode(props.allow);
  const known = createMemo(() => new Set(props.options.map((o) => o.value)));

  const setMode = (next: AccessMode) => {
    setPicked(next);
    if (next === "inherit") props.onChange({ allow: null, deny: props.deny });
    else if (next === "none") props.onChange({ allow: [], deny: props.deny });
    else props.onChange({ allow: props.allow ?? [], deny: props.deny });
  };

  const toggleAllow = (value: string) => {
    const current = props.allow ?? [];
    const next = current.includes(value)
      ? current.filter((v) => v !== value)
      : [...current, value];
    props.onChange({ allow: next, deny: props.deny });
  };

  const toggleDeny = (value: string) => {
    const next = props.deny.includes(value)
      ? props.deny.filter((v) => v !== value)
      : [...props.deny, value];
    props.onChange({ allow: props.allow, deny: next });
  };

  const addPattern = (into: "allow" | "deny") => {
    const raw = pattern().trim();
    if (!raw) return;
    if (into === "allow") {
      const current = props.allow ?? [];
      if (!current.includes(raw)) props.onChange({ allow: [...current, raw], deny: props.deny });
    } else if (!props.deny.includes(raw)) {
      props.onChange({ allow: props.allow, deny: [...props.deny, raw] });
    }
    setPattern("");
  };

  /// Entries configured earlier that no longer match anything on offer —
  /// a server that was removed, or a hand-written glob. Never drop these.
  const customAllow = createMemo(() => (props.allow ?? []).filter((v) => !known().has(v)));
  const customDeny = createMemo(() => props.deny.filter((v) => !known().has(v)));
  const restricted = () => mode() !== "inherit" || props.deny.length > 0;

  return (
    <section class="access-picker" classList={{ restricted: restricted() }}>
      <header class="access-head">
        <div>
          <h4>{props.title}</h4>
          <Show when={props.help}>
            <p class="dim">{props.help}</p>
          </Show>
        </div>
        <Show when={restricted()}>
          <span class="chip chip-tone-warning">restricted</span>
        </Show>
      </header>

      <div class="choice-row" role="radiogroup" aria-label={props.title}>
        <For
          each={
            [
              { id: "inherit" as const, label: props.inheritLabel },
              { id: "limit" as const, label: props.limitLabel },
              { id: "none" as const, label: props.noneLabel },
            ]
          }
        >
          {(choice) => (
            <button
              type="button"
              role="radio"
              aria-checked={mode() === choice.id}
              class="choice"
              classList={{ active: mode() === choice.id }}
              onClick={() => setMode(choice.id)}
            >
              {choice.label}
            </button>
          )}
        </For>
      </div>

      <Show when={mode() === "limit"}>
        <Show
          when={!props.loading}
          fallback={<p class="dim">Looking up the available {props.noun}s…</p>}
        >
          <Show
            when={props.options.length > 0}
            fallback={
              <p class="dim">
                No {props.noun} is configured yet, so there is nothing to pick. Add one first, or
                use a pattern below.
              </p>
            }
          >
            <div class="pick-grid">
              <For each={props.options}>
                {(opt) => (
                  <label class="pick" classList={{ on: (props.allow ?? []).includes(opt.value) }}>
                    <input
                      type="checkbox"
                      checked={(props.allow ?? []).includes(opt.value)}
                      onChange={() => toggleAllow(opt.value)}
                    />
                    <span>
                      <strong>{opt.label}</strong>
                      <Show when={opt.hint}>
                        <em>{opt.hint}</em>
                      </Show>
                    </span>
                  </label>
                )}
              </For>
            </div>
          </Show>
          <Show when={customAllow().length > 0}>
            <div class="chip-stack">
              <For each={customAllow()}>
                {(v) => (
                  <span class="chip mono">
                    {v}
                    <button
                      type="button"
                      class="chip-x"
                      aria-label={`Remove ${v}`}
                      onClick={() => toggleAllow(v)}
                    >
                      ×
                    </button>
                  </span>
                )}
              </For>
            </div>
          </Show>
          <Show when={(props.allow ?? []).length === 0}>
            <p class="dim warn-line">
              Nothing picked yet — as it stands this blocks every {props.noun}.
            </p>
          </Show>
        </Show>
      </Show>

      <Show when={props.deny.length > 0}>
        <div class="deny-line">
          <span class="eyebrow">never allowed</span>
          <div class="chip-stack">
            <For each={props.deny}>
              {(v) => (
                <span class="chip chip-phrase chip-tone-danger">
                  {props.options.find((o) => o.value === v)?.label ?? v}
                  <button
                    type="button"
                    class="chip-x"
                    aria-label={`Stop blocking ${v}`}
                    onClick={() => toggleDeny(v)}
                  >
                    ×
                  </button>
                </span>
              )}
            </For>
          </div>
        </div>
      </Show>

      <button type="button" class="link-button" onClick={() => setShowAdvanced(!showAdvanced())}>
        {showAdvanced()
          ? "Hide advanced"
          : mode() === "limit"
            ? `Block ${props.noun}s, or add one by name`
            : `Block specific ${props.noun}s`}
      </button>

      <Show when={showAdvanced()}>
        <div class="advanced-box">
          <Show when={props.options.length > 0}>
            <div class="pick-grid">
              <For each={props.options}>
                {(opt) => (
                  <label class="pick deny" classList={{ on: props.deny.includes(opt.value) }}>
                    <input
                      type="checkbox"
                      checked={props.deny.includes(opt.value)}
                      onChange={() => toggleDeny(opt.value)}
                    />
                    <span>
                      <strong>{opt.label}</strong>
                    </span>
                  </label>
                )}
              </For>
            </div>
          </Show>
          <Show when={customDeny().length > 0}>
            <div class="chip-stack">
              <For each={customDeny()}>
                {(v) => (
                  <span class="chip mono">
                    {v}
                    <button
                      type="button"
                      class="chip-x"
                      aria-label={`Remove ${v}`}
                      onClick={() => toggleDeny(v)}
                    >
                      ×
                    </button>
                  </span>
                )}
              </For>
            </div>
          </Show>
          <div class="pattern-row">
            <input
              class="mono"
              placeholder={props.patternHint ?? `pattern, e.g. name/*`}
              value={pattern()}
              onInput={(e) => setPattern(e.currentTarget.value)}
              onKeyDown={(e) => e.key === "Enter" && addPattern("deny")}
            />
            <button type="button" class="ghost small" onClick={() => addPattern("deny")}>
              Block
            </button>
            <Show when={mode() === "limit"}>
              <button type="button" class="ghost small" onClick={() => addPattern("allow")}>
                Allow
              </button>
            </Show>
          </div>
          <p class="dim">
            Patterns match by name and accept <code>*</code>. Use one only for something not listed
            above — a {props.noun} that is not registered yet, for instance.
          </p>
        </div>
      </Show>
    </section>
  );
}

// ---- Native <select> value sync --------------------------------------------

/// A native `<select>` applies its `value` when the element is created. Every
/// provider/model list on these screens is fetched, so the options arrive
/// *after* that — the browser has already defaulted to the first option, and
/// the control then shows something different from the state it represents.
/// (Settings displayed "Anthropic" while the configured provider was
/// `openai-responses`; saving would then write a value the operator never
/// saw.) Re-assert the value once the options exist, and again whenever
/// either the value or the list changes.
///
/// Use as a ref: `ref={(el) => syncSelect(el, () => value(), () => list())}`.
/// `createEffect` rather than a render effect, so it always runs after the
/// options have been inserted.
export function syncSelect(
  el: HTMLSelectElement,
  value: () => string,
  options: () => unknown,
) {
  createEffect(() => {
    options();
    const want = value();
    if (want !== "" && el.value !== want) el.value = want;
  });
}

// ---- Durations -------------------------------------------------------------

/// Seconds, said the way a person would. Rounding to whole minutes turned
/// 30s into "1 minutes" and an hour into "60 minutes", so pick the unit from
/// the size of the value and get the plural right.
export function describeDuration(seconds: number | null | undefined): string {
  const s = Math.max(0, Math.round(seconds ?? 0));
  const plural = (n: number, unit: string) => `${n} ${unit}${n === 1 ? "" : "s"}`;
  if (s < 60) return plural(s, "second");
  if (s < 3600) return plural(Math.round(s / 60), "minute");
  const hours = Math.floor(s / 3600);
  const minutes = Math.round((s % 3600) / 60);
  if (minutes === 0) return plural(hours, "hour");
  if (minutes === 60) return plural(hours + 1, "hour");
  return `${plural(hours, "hour")} ${plural(minutes, "minute")}`;
}

/// Compact form for a table cell, where the row height is the constraint.
export function shortDuration(seconds: number | null | undefined): string {
  const s = Math.max(0, Math.round(seconds ?? 0));
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.round(s / 60)}m`;
  const hours = Math.floor(s / 3600);
  const minutes = Math.round((s % 3600) / 60);
  return minutes ? `${hours}h ${minutes}m` : `${hours}h`;
}

// ---- Schedule builder ------------------------------------------------------

export type SchedulePresetId =
  | "every-15m"
  | "every-30m"
  | "hourly"
  | "daily"
  | "weekdays"
  | "weekly"
  | "custom";

const DAYS = [
  { value: "1", label: "Monday" },
  { value: "2", label: "Tuesday" },
  { value: "3", label: "Wednesday" },
  { value: "4", label: "Thursday" },
  { value: "5", label: "Friday" },
  { value: "6", label: "Saturday" },
  { value: "0", label: "Sunday" },
];

interface ScheduleParts {
  preset: SchedulePresetId;
  hour: string;
  minute: string;
  day: string;
}

/// Recognise the handful of cron shapes this builder emits so an existing
/// task opens on the preset that produced it rather than on "custom".
export function parseSchedule(cron: string | null | undefined): ScheduleParts {
  const fallback: ScheduleParts = { preset: "daily", hour: "09", minute: "00", day: "1" };
  const raw = (cron ?? "").trim();
  if (!raw) return { ...fallback, preset: "hourly" };
  const f = raw.split(/\s+/);
  if (f.length !== 5) return { ...fallback, preset: "custom" };
  const [min, hour, dom, mon, dow] = f;
  const pad = (v: string) => (v.length < 2 ? `0${v}` : v);
  if (dom === "*" && mon === "*") {
    if (min === "*/15" && hour === "*" && dow === "*") return { ...fallback, preset: "every-15m" };
    if (min === "*/30" && hour === "*" && dow === "*") return { ...fallback, preset: "every-30m" };
    if (min === "0" && hour === "*" && dow === "*") return { ...fallback, preset: "hourly" };
    if (/^\d{1,2}$/.test(min) && /^\d{1,2}$/.test(hour)) {
      if (dow === "*") return { preset: "daily", hour: pad(hour), minute: pad(min), day: "1" };
      if (dow === "1-5") return { preset: "weekdays", hour: pad(hour), minute: pad(min), day: "1" };
      if (/^[0-6]$/.test(dow))
        return { preset: "weekly", hour: pad(hour), minute: pad(min), day: dow };
    }
  }
  return { ...fallback, preset: "custom" };
}

export function scheduleToCron(parts: ScheduleParts): string {
  const m = String(parseInt(parts.minute, 10) || 0);
  const h = String(parseInt(parts.hour, 10) || 0);
  switch (parts.preset) {
    case "every-15m":
      return "*/15 * * * *";
    case "every-30m":
      return "*/30 * * * *";
    case "hourly":
      return "0 * * * *";
    case "daily":
      return `${m} ${h} * * *`;
    case "weekdays":
      return `${m} ${h} * * 1-5`;
    case "weekly":
      return `${m} ${h} * * ${parts.day}`;
    default:
      return "";
  }
}

/// One-line English for a stored cron, for tables and summaries.
export function describeSchedule(cron: string | null | undefined): string {
  const parts = parseSchedule(cron);
  const at = `${parts.hour}:${parts.minute}`;
  switch (parts.preset) {
    case "every-15m":
      return "Every 15 minutes";
    case "every-30m":
      return "Every 30 minutes";
    case "hourly":
      return "Every hour";
    case "daily":
      return `Every day at ${at}`;
    case "weekdays":
      return `Weekdays at ${at}`;
    case "weekly":
      return `Every ${DAYS.find((d) => d.value === parts.day)?.label ?? "week"} at ${at}`;
    default:
      return (cron ?? "").trim() || "Every hour";
  }
}

const PRESETS: { id: SchedulePresetId; label: string }[] = [
  { id: "every-15m", label: "Every 15 minutes" },
  { id: "every-30m", label: "Every 30 minutes" },
  { id: "hourly", label: "Every hour" },
  { id: "daily", label: "Every day" },
  { id: "weekdays", label: "Weekdays" },
  { id: "weekly", label: "Once a week" },
  { id: "custom", label: "Custom schedule" },
];

/// Replaces a bare `*/30 * * * *` field. Emits the same cron string the
/// server already accepts; `custom` hands the raw field back for the rare
/// schedule the presets cannot express.
export function ScheduleBuilder(props: { value: string; onChange: (cron: string) => void }) {
  const initial = parseSchedule(props.value);
  const [preset, setPreset] = createSignal<SchedulePresetId>(initial.preset);
  const [hour, setHour] = createSignal(initial.hour);
  const [minute, setMinute] = createSignal(initial.minute);
  const [day, setDay] = createSignal(initial.day);

  // The preset is a *view* of `props.value`, not a copy of it. A parent that
  // clears the field after saving, or loads a different record into the same
  // mounted form, must not leave the dropdown showing the previous schedule
  // while the value underneath says something else. Track what we last sent
  // so our own emissions don't re-derive (and don't fight a custom cron the
  // operator is halfway through typing).
  let lastEmitted: string | null = null;
  const emitCron = (cron: string) => {
    lastEmitted = cron;
    props.onChange(cron);
  };
  createRenderEffect(() => {
    const incoming = props.value;
    if (incoming === lastEmitted) return;
    lastEmitted = incoming;
    const next = parseSchedule(incoming);
    setPreset(next.preset);
    setHour(next.hour);
    setMinute(next.minute);
    setDay(next.day);
  });

  const emit = () => {
    const next = scheduleToCron({ preset: preset(), hour: hour(), minute: minute(), day: day() });
    if (preset() !== "custom") emitCron(next);
  };

  const pick = (id: SchedulePresetId) => {
    setPreset(id);
    if (id === "custom") return;
    emitCron(scheduleToCron({ preset: id, hour: hour(), minute: minute(), day: day() }));
  };

  const needsTime = () => preset() === "daily" || preset() === "weekdays" || preset() === "weekly";

  return (
    <div class="schedule-builder">
      <div class="form-row">
        <label>How often</label>
        <select value={preset()} onChange={(e) => pick(e.currentTarget.value as SchedulePresetId)}>
          <For each={PRESETS}>{(p) => <option value={p.id}>{p.label}</option>}</For>
        </select>
      </div>
      <Show when={preset() === "weekly"}>
        <div class="form-row">
          <label>On</label>
          <select
            value={day()}
            onChange={(e) => {
              setDay(e.currentTarget.value);
              emit();
            }}
          >
            <For each={DAYS}>{(d) => <option value={d.value}>{d.label}</option>}</For>
          </select>
        </div>
      </Show>
      <Show when={needsTime()}>
        <div class="form-row">
          <label>At</label>
          <input
            type="time"
            value={`${hour()}:${minute()}`}
            onInput={(e) => {
              const [h, m] = e.currentTarget.value.split(":");
              if (h && m) {
                setHour(h);
                setMinute(m);
                emit();
              }
            }}
          />
        </div>
      </Show>
      <Show when={preset() === "custom"}>
        <div class="form-row">
          <label>Cron</label>
          <input
            class="mono"
            placeholder="*/30 * * * *"
            value={props.value}
            onInput={(e) => emitCron(e.currentTarget.value)}
          />
        </div>
        <p class="dim">
          Five fields — minute, hour, day of month, month, day of week. Leave the field empty to
          run hourly.
        </p>
      </Show>
      <Show when={preset() !== "custom"}>
        <p class="dim">Runs {describeSchedule(props.value).toLowerCase()}.</p>
      </Show>
    </div>
  );
}

// ---- Hook matcher builder --------------------------------------------------

export interface MatcherTool {
  value: string;
  label: string;
}

interface MatcherParts {
  scope: "all" | "tool" | "custom";
  tool: string;
  args: string;
}

/// `Bash(git *)` → the tool and its argument pattern; a bare `Write` → just
/// the tool; anything else stays custom so it round-trips untouched.
export function parseMatcher(raw: string | null | undefined): MatcherParts {
  const value = (raw ?? "").trim();
  if (!value) return { scope: "all", tool: "", args: "" };
  const m = /^([A-Za-z0-9_-]+)\((.*)\)$/.exec(value);
  if (m) return { scope: "tool", tool: m[1], args: m[2] };
  if (/^[A-Za-z0-9_-]+$/.test(value)) return { scope: "tool", tool: value, args: "" };
  return { scope: "custom", tool: "", args: "" };
}

export function matcherToString(parts: MatcherParts, custom: string): string {
  if (parts.scope === "all") return "";
  if (parts.scope === "custom") return custom.trim();
  if (!parts.tool) return "";
  return parts.args.trim() ? `${parts.tool}(${parts.args.trim()})` : parts.tool;
}

/// Plain reading of a stored matcher, for the hooks table.
export function describeMatcher(raw: string | null | undefined): string {
  const parts = parseMatcher(raw);
  if (parts.scope === "all") return "Every tool call";
  if (parts.scope === "custom") return (raw ?? "").trim();
  return parts.args ? `${parts.tool} matching ${parts.args}` : `Every ${parts.tool} call`;
}

/// Replaces the free-text "matcher" field. A hook still stores one string;
/// this only stops the operator having to know its grammar.
export function MatcherBuilder(props: {
  value: string;
  onChange: (matcher: string) => void;
  tools: MatcherTool[];
}) {
  const initial = parseMatcher(props.value);
  const [scope, setScope] = createSignal(initial.scope);
  const [tool, setTool] = createSignal(initial.tool || props.tools[0]?.value || "bash");
  const [args, setArgs] = createSignal(initial.args);
  const [custom, setCustom] = createSignal(initial.scope === "custom" ? props.value : "");

  // Same contract as ScheduleBuilder: these controls display `props.value`,
  // so a parent clearing it after a save (or loading a different hook into an
  // already-mounted form) has to be reflected here rather than leaving the
  // dropdowns describing a matcher that is no longer the value.
  let lastEmitted: string | null = null;
  const emitMatcher = (matcher: string) => {
    lastEmitted = matcher;
    props.onChange(matcher);
  };
  createRenderEffect(() => {
    const incoming = props.value;
    if (incoming === lastEmitted) return;
    lastEmitted = incoming;
    const next = parseMatcher(incoming);
    setScope(next.scope);
    if (next.tool) setTool(next.tool);
    setArgs(next.args);
    setCustom(next.scope === "custom" ? incoming : "");
  });

  const emit = (next?: Partial<MatcherParts>) => {
    const parts: MatcherParts = {
      scope: next?.scope ?? scope(),
      tool: next?.tool ?? tool(),
      args: next?.args ?? args(),
    };
    emitMatcher(matcherToString(parts, custom()));
  };

  return (
    <div class="matcher-builder">
      <div class="form-row">
        <label>Runs on</label>
        <select
          value={scope()}
          onChange={(e) => {
            const v = e.currentTarget.value as MatcherParts["scope"];
            setScope(v);
            emit({ scope: v });
          }}
        >
          <option value="all">Every tool call</option>
          <option value="tool">One specific tool</option>
          <option value="custom">A pattern I write myself</option>
        </select>
      </div>
      <Show when={scope() === "tool"}>
        <div class="form-row">
          <label>Tool</label>
          <select
            value={tool()}
            onChange={(e) => {
              setTool(e.currentTarget.value);
              emit({ tool: e.currentTarget.value });
            }}
          >
            <For each={props.tools}>{(t) => <option value={t.value}>{t.label}</option>}</For>
          </select>
        </div>
        <div class="form-row">
          <label>Only when it starts with</label>
          <input
            class="mono"
            placeholder="optional — e.g. git"
            value={args()}
            onInput={(e) => {
              setArgs(e.currentTarget.value);
              emit({ args: e.currentTarget.value });
            }}
          />
        </div>
        <p class="dim">
          Leave the second field empty to run on every {tool()} call. <code>*</code> stands for
          anything.
        </p>
      </Show>
      <Show when={scope() === "custom"}>
        <div class="form-row">
          <label>Pattern</label>
          <input
            class="mono"
            placeholder="mcp(github/*)"
            value={custom()}
            onInput={(e) => {
              setCustom(e.currentTarget.value);
              emitMatcher(e.currentTarget.value.trim());
            }}
          />
        </div>
      </Show>
    </div>
  );
}

// ---- Shared option sets ----------------------------------------------------

/// The built-in tools, by the name the permission engine matches (which is
/// case-insensitive, so the lowercase wire name is what we store).
export const BUILTIN_TOOLS: AccessOption[] = [
  { value: "read", label: "Read files", hint: "read" },
  { value: "grep", label: "Search file contents", hint: "grep" },
  { value: "glob", label: "Find files by name", hint: "glob" },
  { value: "write", label: "Create files", hint: "write" },
  { value: "edit", label: "Edit files", hint: "edit" },
  { value: "bash", label: "Run shell commands", hint: "bash" },
  { value: "webfetch", label: "Fetch a web page", hint: "webfetch" },
  { value: "browse", label: "Browse the web", hint: "browse" },
];

export const MATCHER_TOOLS: MatcherTool[] = BUILTIN_TOOLS.map((t) => ({
  value: t.value,
  label: `${t.label} (${t.value})`,
}));
