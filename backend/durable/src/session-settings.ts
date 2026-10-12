import { DEFAULT_COMPACTION_POLICY, defineDoc, type CompactionPolicy, type Conversation } from "@earendil-works/pi-durable";
import type { Context } from "@earendil-works/chord";

/** Conversation-owned settings. Omitted optional fields retain older sessions' defaults. */
export const Metadata = defineDoc<{ name: string; autoCompaction: boolean; initialized: boolean; swarmEnabled?: boolean; compactionThresholdPercent?: number }>({
  kind: "app.desktop", version: 1, scope: "conversation", history: "latest", fork: "initial",
  initial: () => ({ name: "", autoCompaction: true, initialized: false, swarmEnabled: true }),
});

/** Resolve a background-start percentage while preserving summary/output headroom.
 * A smaller model may clamp the saved intent to its earlier blocking limit.
 * Missing intent (including old sessions) preserves the upstream token defaults.
 */
export function compactionPolicy(
  metadata: { autoCompaction: boolean; compactionThresholdPercent?: number },
  contextWindow: number | undefined,
): CompactionPolicy {
  const policy = { ...DEFAULT_COMPACTION_POLICY, enabled: metadata.autoCompaction };
  const percent = metadata.compactionThresholdPercent;
  if (percent === undefined || contextWindow === undefined || !Number.isSafeInteger(contextWindow) || contextWindow <= 0) return policy;
  const trigger = Math.round(contextWindow * percent / 100);
  policy.backgroundTokens = Math.max(0, contextWindow - policy.reserveTokens - trigger);
  return policy;
}

export async function setCompactionThreshold(root: Conversation, percent: unknown, context: Context): Promise<{ percent: number | null }> {
  if (percent !== null && (typeof percent !== "number" || !Number.isFinite(percent) || percent < 10 || percent > 95)) {
    throw new Error("percent must be a finite number between 10 and 95, or null to restore the default");
  }
  await root.commit(async (tx) => {
    const metadata = await tx.doc(Metadata, root.id);
    if (percent === null) delete metadata.compactionThresholdPercent;
    else metadata.compactionThresholdPercent = percent;
  }, context);
  return { percent };
}

export const swarmEnabled = (metadata: { swarmEnabled?: boolean } | null | undefined): boolean => metadata?.swarmEnabled !== false;

export async function setSwarmEnabled(root: Conversation, enabled: unknown, context: Context): Promise<{ enabled: boolean }> {
  if (typeof enabled !== "boolean") throw new Error("enabled must be a boolean");
  await root.commit(async (tx) => { (await tx.doc(Metadata, root.id)).swarmEnabled = enabled; }, context);
  return { enabled };
}
