import { Match, Switch } from "solid-js";

export default function AgentMark(props: { character?: string; size?: number; working?: boolean; class?: string }) {
  const character = () => ["orb", "leaf", "sun", "wave", "spark"].includes(props.character ?? "") ? props.character : "orb";
  return <span class={`agent-mark ${character()} ${props.working ? "working" : ""} ${props.class ?? ""}`} style={{ width: `${props.size ?? 26}px`, height: `${props.size ?? 26}px` }} aria-hidden="true">
    <svg viewBox="0 0 40 40" fill="none" xmlns="http://www.w3.org/2000/svg">
      <Switch>
        <Match when={character() === "orb"}>
          <path d="M20 4C11 4 5 11 5 20s6 16 15 16 15-7 15-16S29 4 20 4Z" fill="currentColor" opacity=".18" />
          <path d="M28 8c-10-1-17 5-17 13 0 6 4 10 10 11" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" />
          <circle cx="24" cy="18" r="1.3" fill="currentColor" /><circle cx="29" cy="20" r="1.3" fill="currentColor" />
        </Match>
        <Match when={character() === "leaf"}>
          <path d="M32 6C15 7 7 14 8 25c1 7 6 10 12 9 12-2 15-13 12-28Z" fill="currentColor" opacity=".2" />
          <path d="M7 35c7-10 15-17 25-29" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" />
          <circle cx="21" cy="20" r="1.2" fill="currentColor" /><circle cx="26" cy="18" r="1.2" fill="currentColor" />
        </Match>
        <Match when={character() === "sun"}>
          <path d="M20 7c8 0 13 5 13 13s-5 13-13 13S7 28 7 20 12 7 20 7Z" fill="currentColor" opacity=".2" />
          <path d="M20 2v4M20 34v4M2 20h4M34 20h4M7 7l3 3M30 30l3 3M33 7l-3 3M10 30l-3 3" stroke="currentColor" stroke-width="2" stroke-linecap="round" />
          <circle cx="16" cy="18" r="1.3" fill="currentColor" /><circle cx="24" cy="18" r="1.3" fill="currentColor" />
          <path d="M16 24c2 2 6 2 8 0" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" />
        </Match>
        <Match when={character() === "wave"}>
          <path d="M3 24c7-16 13-17 20-5 5 8 8 8 14-2v12c-6 8-12 8-18 0-5-6-10-7-16 2V24Z" fill="currentColor" opacity=".18" />
          <path d="M3 24c7-16 13-17 20-5 5 8 8 8 14-2" stroke="currentColor" stroke-width="2.7" stroke-linecap="round" />
          <circle cx="15" cy="19" r="1.2" fill="currentColor" /><circle cx="20" cy="20" r="1.2" fill="currentColor" />
        </Match>
        <Match when={character() === "spark"}>
          <path d="M20 3c2 10 5 15 17 17-12 2-15 7-17 17C18 27 15 22 3 20 15 18 18 13 20 3Z" fill="currentColor" opacity=".22" />
          <path d="M20 3c2 10 5 15 17 17-12 2-15 7-17 17C18 27 15 22 3 20 15 18 18 13 20 3Z" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round" />
          <circle cx="17" cy="19" r="1" fill="currentColor" /><circle cx="23" cy="19" r="1" fill="currentColor" />
        </Match>
      </Switch>
    </svg>
  </span>;
}
