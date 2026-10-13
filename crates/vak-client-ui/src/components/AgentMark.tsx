import { createSignal, onCleanup, onMount, Show } from "solid-js";
import { agentCharacter, AGENT_CHARACTERS, characterAtlas, characterLoop, characterPortrait, type AgentCharacter } from "../agentGlyph";
import { characterTier } from "../characterTier";
import { uiPreferences } from "../store";

export type CharacterState = "idle" | "listening" | "thinking" | "working" | "waiting" | "success" | "concern" | "acknowledge";
const ACTIVE_STATES = new Set<CharacterState>(["listening", "thinking", "working", "waiting"]);
const reducedMotionQuery = typeof matchMedia === "undefined" ? undefined : matchMedia("(prefers-reduced-motion: reduce)");
const [systemReducesMotion, setSystemReducesMotion] = createSignal(reducedMotionQuery?.matches ?? false);
reducedMotionQuery?.addEventListener("change", (event) => setSystemReducesMotion(event.matches));

export default function AgentMark(props: {
  character?: string;
  size?: number;
  state?: CharacterState;
  motion?: "subtle" | "expressive" | "off";
  interactive?: boolean;
  class?: string;
}) {
  const [reaction, setReaction] = createSignal(false);
  const [visible, setVisible] = createSignal(false);
  const [atlasFailed, setAtlasFailed] = createSignal(false);
  const [dimensionalFailed, setDimensionalFailed] = createSignal(false);
  const [loopReady, setLoopReady] = createSignal(false);
  let mark: HTMLSpanElement | undefined;
  let reactionTimer: number | undefined;
  const id = () => (props.character && props.character in AGENT_CHARACTERS ? props.character : "vak") as AgentCharacter;
  const companion = () => agentCharacter(id());
  const state = () => reaction() ? "acknowledge" : props.state ?? "idle";
  const tier = () => characterTier((props.size ?? 26) * (typeof window === "undefined" ? 1 : window.devicePixelRatio || 1));
  const portrait = () => characterPortrait(id(), tier());
  const atlas = () => characterAtlas(id(), tier());
  const dimensional = () => `${import.meta.env.BASE_URL}assets/brand/dimensional/${(props.size ?? 26) <= 32 ? "glyphs" : "characters"}/${id()}-${(props.size ?? 26) <= 32 ? Math.min(128, Math.max(32, tier())) : tier()}.webp`;
  // The loop plays only while the character is doing something and on screen;
  // idle and off-screen marks keep the still, so a long chat never decodes
  // dozens of animations.
  const looping = () => uiPreferences.visualPack === "dimensional" && visible() && props.motion !== "off"
    && !uiPreferences.reduceMotion && !systemReducesMotion() && ACTIVE_STATES.has(state());
  const acknowledge = () => {
    if (!props.interactive) return;
    setReaction(true);
    window.clearTimeout(reactionTimer);
    reactionTimer = window.setTimeout(() => setReaction(false), 620);
  };
  onCleanup(() => window.clearTimeout(reactionTimer));
  onMount(() => {
    if (!mark || typeof IntersectionObserver === "undefined") {
      setVisible(true);
      return;
    }
    const observer = new IntersectionObserver(([entry]) => setVisible(entry.isIntersecting));
    observer.observe(mark);
    onCleanup(() => observer.disconnect());
  });

  return <span ref={mark}
    class={`agent-mark companion ${id()} state-${state()} motion-${props.motion ?? "subtle"} ${visible() ? "is-visible" : ""} ${atlasFailed() ? "atlas-failed" : ""} ${dimensionalFailed() ? "dimensional-failed" : ""} ${looping() && loopReady() ? "has-loop" : ""} ${props.interactive ? "interactive" : ""} ${props.class ?? ""}`}
    style={{ width: `${props.size ?? 26}px`, height: `${props.size ?? 26}px`, "--companion-hue": `${companion().hue}` }}
    data-character-state={state()}
    onPointerDown={acknowledge}
    aria-hidden="true"
  >
    <img class="agent-mark-fallback" src={portrait()} alt="" draggable={false} />
    <Show when={uiPreferences.visualPack === "dimensional"}><img class="agent-mark-dimensional" src={dimensional()} alt="" draggable={false} onError={() => setDimensionalFailed(true)} onLoad={() => setDimensionalFailed(false)} /></Show>
    <img class="agent-mark-atlas-source" src={atlas()} alt="" onError={() => setAtlasFailed(true)} onLoad={() => setAtlasFailed(false)} />
    <span class="agent-mark-atlas" style={{ "background-image": `url(${atlas()})` }} />
    <Show when={looping()}><img class="agent-mark-loop" src={characterLoop(id(), tier())} alt="" draggable={false} onLoad={() => setLoopReady(true)} onError={() => setLoopReady(false)} /></Show>
  </span>;
}
