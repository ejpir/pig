// Offline editor preview. This is not a live project file.
interface ThinkingBlock {
  type: "thinking";
  signature?: string;
}
type Model = { provider: string };
declare function isAnthropic(model: Model): boolean;

export function readThinking(blocks: ThinkingBlock[], model: Model) {
  const signatures: string[] = [];
  for (const block of blocks) {
    if (block.type === "thinking") {
      const signature = block.signature;
      if (!signature && isAnthropic(model)) {
        throw new Error("Missing signature");
      }
      signatures.push(signature ?? "");
    }
  }
  return signatures;
}
