import { createSignal, onCleanup, onMount } from "solid-js";
import { agentCharacter, AGENT_CHARACTERS, type AgentCharacter } from "../agentGlyph";

export type CharacterState = "idle" | "listening" | "thinking" | "working" | "waiting" | "success" | "concern" | "acknowledge";

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
  let mark: HTMLSpanElement | undefined;
  let reactionTimer: number | undefined;
  const id = () => (props.character && props.character in AGENT_CHARACTERS ? props.character : "vak") as AgentCharacter;
  const companion = () => agentCharacter(id());
  const state = () => reaction() ? "acknowledge" : props.state ?? "idle";
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
    class={`agent-mark companion ${id()} state-${state()} motion-${props.motion ?? "subtle"} ${visible() ? "is-visible" : ""} ${atlasFailed() ? "atlas-failed" : ""} ${props.interactive ? "interactive" : ""} ${props.class ?? ""}`}
    style={{ width: `${props.size ?? 26}px`, height: `${props.size ?? 26}px`, "--companion-hue": `${companion().hue}` }}
    data-character-state={state()}
    onPointerDown={acknowledge}
    aria-hidden="true"
  >
    <img class="agent-mark-fallback" src={companion().image} alt="" draggable={false} />
    <img class="agent-mark-atlas-source" src={companion().atlas} alt="" onError={() => setAtlasFailed(true)} onLoad={() => setAtlasFailed(false)} />
    <span class="agent-mark-atlas" style={{ "background-image": `url(${companion().atlas})` }} />
  </span>;
}
