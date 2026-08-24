import type { JSX } from "solid-js";

export type IconName =
  | "add"
  | "archive"
  | "branch"
  | "chat"
  | "check"
  | "chevron"
  | "close"
  | "code"
  | "diff"
  | "folder"
  | "gear"
  | "git"
  | "grid"
  | "history"
  | "layers"
  | "palette"
  | "plug"
  | "preview"
  | "receipt"
  | "restore"
  | "search"
  | "shield"
  | "send"
  | "sidebar"
  | "spark"
  | "stop"
  | "terminal"
  | "trash"
  | "timer"
  | "tune";

const paths: Record<IconName, () => JSX.Element> = {
  add: () => <><path d="M12 5v14M5 12h14" /></>,
  archive: () => <><rect x="3" y="4" width="18" height="5" rx="1" /><path d="M5 9v10a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1V9M10 13h4" /></>,
  branch: () => <><circle cx="6" cy="5" r="2" /><circle cx="18" cy="6" r="2" /><circle cx="6" cy="19" r="2" /><path d="M6 7v10M8 9c5 0 5-3 8-3" /></>,
  chat: () => <path d="M20 15a3 3 0 0 1-3 3H9l-5 3v-6a3 3 0 0 1-1-2V7a3 3 0 0 1 3-3h11a3 3 0 0 1 3 3Z" />,
  check: () => <path d="m5 12 4 4L19 6" />,
  chevron: () => <path d="m9 18 6-6-6-6" />,
  close: () => <><path d="m6 6 12 12M18 6 6 18" /></>,
  code: () => <><path d="m8 9-3 3 3 3M16 9l3 3-3 3M14 5l-4 14" /></>,
  diff: () => <><path d="M7 3v18M17 3v18M4 7h6M14 17h6M17 7v6M14 10h6" /></>,
  folder: () => <path d="M3 7a2 2 0 0 1 2-2h5l2 2h7a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2Z" />,
  gear: () => <><circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.9l.1.1-2.8 2.8-.1-.1a1.7 1.7 0 0 0-1.9-.3 1.7 1.7 0 0 0-1 1.6v.2h-4V21a1.7 1.7 0 0 0-1-1.6 1.7 1.7 0 0 0-1.9.3l-.1.1L4.2 17l.1-.1a1.7 1.7 0 0 0 .3-1.9A1.7 1.7 0 0 0 3 14H2.8v-4H3a1.7 1.7 0 0 0 1.6-1 1.7 1.7 0 0 0-.3-1.9L4.2 7 7 4.2l.1.1A1.7 1.7 0 0 0 9 4.6 1.7 1.7 0 0 0 10 3v-.2h4V3a1.7 1.7 0 0 0 1 1.6 1.7 1.7 0 0 0 1.9-.3l.1-.1L19.8 7l-.1.1a1.7 1.7 0 0 0-.3 1.9 1.7 1.7 0 0 0 1.6 1h.2v4H21a1.7 1.7 0 0 0-1.6 1Z" /></>,
  git: () => <><circle cx="6" cy="5" r="2" /><circle cx="18" cy="5" r="2" /><circle cx="6" cy="19" r="2" /><path d="M6 7v10M8 11h4a6 6 0 0 0 6-6" /></>,
  grid: () => <><rect x="4" y="4" width="6" height="6" rx="1" /><rect x="14" y="4" width="6" height="6" rx="1" /><rect x="4" y="14" width="6" height="6" rx="1" /><rect x="14" y="14" width="6" height="6" rx="1" /></>,
  history: () => <><path d="M3.5 12a8.5 8.5 0 1 0 2.5-6L3.5 8.5" /><path d="M3.5 4v4.5H8" /><path d="M12 8v4l2.5 2" /></>,
  layers: () => <><path d="m12 3 9 5-9 5-9-5Z" /><path d="m3 12 9 5 9-5M3 16l9 5 9-5" /></>,
  palette: () => <><path d="M12 3a9 9 0 0 0 0 18h1.5a1.5 1.5 0 0 0 0-3H13a2 2 0 0 1 0-4h2a6 6 0 0 0 6-6c0-3-4-5-9-5Z" /><circle cx="7.5" cy="10.5" r=".7" /><circle cx="10" cy="7" r=".7" /><circle cx="15" cy="7" r=".7" /></>,
  plug: () => <><path d="m8 12 8-8M14 3l7 7M5 13l6 6M3 21l5-5M16 8l-5 5" /></>,
  preview: () => <><circle cx="12" cy="12" r="3" /><path d="M2.5 12s3.5-6 9.5-6 9.5 6 9.5 6-3.5 6-9.5 6-9.5-6-9.5-6Z" /></>,
  restore: () => <><path d="M3.5 12a8.5 8.5 0 1 0 2.5-6L3.5 8.5" /><path d="M3.5 4v4.5H8" /><path d="m9 12 2 2 4-4" /></>,
  search: () => <><circle cx="11" cy="11" r="7" /><path d="m20 20-4-4" /></>,
  shield: () => <path d="M12 3 5 6v5c0 4.5 2.8 8 7 10 4.2-2 7-5.5 7-10V6Z" />,
  send: () => <><path d="m5 12 14-7-4 14-3-6Z" /><path d="m12 13 7-8" /></>,
  sidebar: () => <><rect x="3" y="4" width="18" height="16" rx="2" /><path d="M9 4v16" /></>,
  spark: () => <><path d="m12 3 1.4 4.6L18 9l-4.6 1.4L12 15l-1.4-4.6L6 9l4.6-1.4Z" /><path d="m18 15 .7 2.3L21 18l-2.3.7L18 21l-.7-2.3L15 18l2.3-.7Z" /></>,
  stop: () => <rect x="7" y="7" width="10" height="10" rx="2" />,
  terminal: () => <><path d="m5 7 5 5-5 5M12 17h7" /></>,
  trash: () => <><path d="M4 7h16M10 11v5M14 11v5M6 7l1 13h10l1-13M9 7V4h6v3" /></>,
  timer: () => <><circle cx="12" cy="13" r="8" /><path d="M12 9v4l3 2M9 2h6" /></>,
  receipt: () => <><path d="M6 3h12v18l-2.5-1.5L13 21l-2.5-1.5L8 21l-2-1.5Z" /><path d="M9 8h6M9 12h6M9 16h3" /></>,
  tune: () => <><path d="M4 7h10M18 7h2M4 17h2M10 17h10" /><circle cx="16" cy="7" r="2" /><circle cx="8" cy="17" r="2" /></>,
};

export default function Icon(props: { name: IconName; size?: number; class?: string }) {
  return (
    <svg
      class={props.class ?? "icon"}
      width={props.size ?? 16}
      height={props.size ?? 16}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="1.8"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      {paths[props.name]()}
    </svg>
  );
}
