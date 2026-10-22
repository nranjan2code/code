import type { ConversationSnapshot } from "./api";
import type { Message, OutputTimeline, TranscriptEntryMeta } from "./types";

export type HistoryPage = NonNullable<ConversationSnapshot["page"]>;
export type LoadedHistory = ConversationSnapshot & { page: HistoryPage };

/** Reloadable pages, separate from the live turn. Eviction drops the side
 * furthest from the reader; canonical identities survive prepend/reload. */
export class HistoryPages {
  private pages: LoadedHistory[] = [];
  private sizes = new WeakMap<LoadedHistory, number>();
  private readonly byteBudget: number;
  constructor(byteBudget = 8 * 1024 * 1024) { this.byteBudget = byteBudget; }

  get revision(): string | undefined { return this.pages.at(-1)?.revision; }
  get older(): string | null { return this.pages[0]?.page.older ?? null; }
  get newer(): string | null { return this.pages.at(-1)?.page.newer ?? null; }
  get totalTurns(): number { return this.pages.at(-1)?.page.total_turns ?? 0; }
  get size(): number { return this.pages.length; }
  get requests(): (string | undefined)[] { return this.pages.map(page => page.page.request ?? undefined); }
  itemIds(): Set<string> { return new Set(this.pages.flatMap(page => page.presentation.items.map(item => item.id))); }

  add(incoming: LoadedHistory, direction: "older" | "newer" | "refresh" | "update"): void {
    this.sizes.set(incoming, new TextEncoder().encode(JSON.stringify(incoming)).byteLength);
    const index = this.pages.findIndex(page => page.page.end === incoming.page.end || page.page.request === incoming.page.request);
    if (direction === "refresh") this.pages = [incoming];
    else if (index >= 0) this.pages[index] = incoming;
    else if (direction === "older") this.pages.unshift(incoming);
    else this.pages.push(incoming);
    while (this.pages.length > 1 && this.pages.reduce((sum, page) => sum + (this.sizes.get(page) ?? 0), 0) > this.byteBudget) {
      if (direction === "older") this.pages.pop(); else this.pages.shift();
    }
  }

  merged(): { messages: Message[]; entries: TranscriptEntryMeta[]; presentation: OutputTimeline; setAside: string[] } | null {
    const latest = this.pages.at(-1);
    if (!latest) return null;
    const messages = new Map<string, { message: Message; entry: TranscriptEntryMeta }>();
    const items = new Map<string, OutputTimeline["items"][number]>();
    const aside = new Set<string>();
    for (const page of this.pages) {
      for (const [index, message] of page.transcript.messages.entries()) {
        const entry = page.transcript.entries?.[index];
        if (entry) messages.set(entry.entry_id, { message, entry });
      }
      for (const item of page.presentation.items) items.set(item.id, item);
      for (const turn of page.transcript.set_aside ?? []) aside.add(turn);
    }
    return { messages: [...messages.values()].map(row => row.message), entries: [...messages.values()].map(row => row.entry),
      presentation: { ...latest.presentation, items: [...items.values()] }, setAside: [...aside] };
  }
}
