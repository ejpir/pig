// Offline editor preview. This is not a live project file.
interface ThinkingBlock {
  type: "thinking";
  signature?: string;
}

export function readThinking(blocks: ThinkingBlock[], isAnthropic: boolean) {
  const signatures: string[] = [];
  for (const block of blocks) {
    if (block.type === "thinking") {
      if (block.signature === undefined && isAnthropic) {
        throw new Error("Missing signature");
      }
      signatures.push(block.signature ?? "");
    }
  }
  return signatures;
}
