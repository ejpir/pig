/**
 * A conversation's whole transcript, as the apps show it. A conversation's view
 * starts at its latest compaction: the summary, then what it kept. The entries
 * before stay in storage, unchanging, and are read from there once per
 * compaction.
 */
import type { Context } from "@earendil-works/chord";
import type { ConversationView, EntryRecord } from "@earendil-works/pi-durable";

const PAGE = 500;
const SUMMARY = /<summary>\n?([\s\S]*?)\n?<\/summary>/;

type Scan = { entries(query: object, limit: number, cursor: never, context: Context): Promise<{ items: readonly EntryRecord[]; next?: unknown }> };

export class History {
  readonly #known = new Map<number, EntryRecord>();
  /** The head markers whose earlier entries were read. */
  readonly #read = new Set<number>();

  constructor(readonly conversation: Scan, readonly context: Context) {}

  /** Every entry so far, oldest first, with each compaction where it happened. */
  async entries(view: ConversationView): Promise<EntryRecord[]> {
    const head = view.entries[0]?.head !== undefined ? Number(view.entries[0].id) : undefined;
    if (head !== undefined && !this.#read.has(head)) {
      let cursor: unknown;
      do {
        const page = await this.conversation.entries({}, PAGE, cursor as never, this.context);
        for (const entry of page.items) if (!this.#known.has(Number(entry.id))) this.#known.set(Number(entry.id), entry);
        cursor = page.next;
      } while (cursor !== undefined);
      this.#read.add(head);
    }
    // Without a compaction the view holds it all; nothing is kept twice then.
    if (!this.#read.size) return view.entries.map(shown);
    for (const entry of view.entries) this.#known.set(Number(entry.id), entry);
    return [...this.#known.values()].sort((a, b) => Number(a.id) - Number(b.id)).map(shown);
  }
}

/** A compaction reads as stock Pi's `compactionSummary`, not as something the user said. */
function shown(entry: EntryRecord): EntryRecord {
  if (entry.kind !== "pi.compaction") return entry;
  const message = entry.model?.[0] as { content?: { type: string; text?: string }[] | string; timestamp?: number } | undefined;
  const text = typeof message?.content === "string" ? message.content
    : (message?.content ?? []).map((block) => block.type === "text" ? block.text ?? "" : "").join("");
  const summary = SUMMARY.exec(text)?.[1] ?? text;
  return { ...entry, model: [{ role: "compactionSummary", summary, timestamp: message?.timestamp } as never] };
}
