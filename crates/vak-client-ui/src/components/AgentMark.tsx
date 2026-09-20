import { agentCharacter, AGENT_CHARACTERS, type AgentCharacter } from "../agentGlyph";

export default function AgentMark(props: { character?: string; size?: number; working?: boolean; class?: string }) {
  const id = () => (props.character && props.character in AGENT_CHARACTERS ? props.character : "vak") as AgentCharacter;
  const companion = () => agentCharacter(id());
  return <span
    class={`agent-mark companion ${id()} ${props.working ? "working" : "idle"} ${props.class ?? ""}`}
    style={{ width: `${props.size ?? 26}px`, height: `${props.size ?? 26}px`, "--companion-hue": `${companion().hue}` }}
    aria-hidden="true"
  >
    <span class="agent-mark-motion"><img src={companion().image} alt="" draggable={false} /></span>
  </span>;
}
