import { expect, test } from "bun:test";
import { BACKGROUND_CONTEXT } from "@earendil-works/chord/context";
import type { ConversationView, EntryRecord } from "@earendil-works/pi-durable";
import { History } from "../src/history.ts";

const entry = (id: number, text: string, extra: Partial<EntryRecord> = {}) =>
  ({ id, conversationId: 1, kind: "pi.user", model: [{ role: "user", content: text, timestamp: id }], ...extra }) as unknown as EntryRecord;
const view = (entries: EntryRecord[]) => ({ conversation: { id: 1 }, entries, docs: {} }) as unknown as ConversationView;

test("a compaction keeps what came before it, and reads as a summary where it happened", async () => {
  const stored = [entry(1, "first"), entry(2, "second"), entry(3, "kept"),
    entry(4, "", { kind: "pi.compaction", head: 3 as never, model: [{ role: "user", content: [{ type: "text", text: "The conversation history before this point was compacted into the following summary:\n\n<summary>\nDid things.\n</summary>" }], timestamp: 4 }] as never }),
    entry(5, "after")];
  let scans = 0;
  // Newest first, two to a page.
  const conversation = { entries: async (_query: object, limit: number, cursor: number | undefined) => {
    scans++;
    const from = cursor ?? 0;
    const newest = [...stored].reverse();
    return { items: newest.slice(from, from + Math.min(limit, 2)), ...(from + 2 < newest.length ? { next: from + 2 } : {}) };
  } };
  const history = new History(conversation as never, BACKGROUND_CONTEXT);
  // Before any compaction, the view is all of it and storage isn't read.
  expect((await history.entries(view(stored.slice(0, 2)))).map((e) => Number(e.id))).toEqual([1, 2]);
  expect(scans).toBe(0);

  const compacted = view([stored[3], stored[2], stored[4]]);
  const shown = await history.entries(compacted);
  expect(shown.map((e) => Number(e.id))).toEqual([1, 2, 3, 4, 5]);
  expect(shown[3].model).toEqual([{ role: "compactionSummary", summary: "Did things.", timestamp: 4 } as never]);
  const read = scans;
  // Later snapshots of the same compaction add what is new without reading again.
  const later = await history.entries(view([stored[3], stored[2], stored[4], entry(6, "next")]));
  expect(later.map((e) => Number(e.id))).toEqual([1, 2, 3, 4, 5, 6]);
  expect(scans).toBe(read);
});
