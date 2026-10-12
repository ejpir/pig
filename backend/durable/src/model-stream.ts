/** Project provider streams to public assistant messages before durable persists their partials. */
import type { AssistantMessage, AssistantMessageEvent, AssistantMessageEventStream, Models, TextContent, ThinkingContent, ToolCall } from "@earendil-works/pi-ai";

function publicToolCall(block: ToolCall): ToolCall {
  // pi-ai's providers keep JSON parsing/custom-tool buffers and Anthropic's
  // content index on live tool blocks. They are not public ToolCall fields.
  // In pi-durable 1.1.0 their appends trigger watchEvents' whole-block fallback,
  // resending both the parsed arguments and the growing raw JSON every tick.
  // Only omit these direct bookkeeping fields, never similarly named arguments
  // or public metadata such as namespace and thoughtSignature.
  const { partialJson, partialArgs, customInput, index, streamIndex, ...publicBlock } = block as ToolCall & {
    partialJson?: unknown; partialArgs?: unknown; customInput?: unknown; index?: unknown; streamIndex?: unknown;
  };
  return publicBlock;
}

function publicContent(block: AssistantMessage["content"][number]): AssistantMessage["content"][number] {
  if (block.type === "toolCall") return publicToolCall(block);
  if (block.type === "thinking") {
    // Bedrock accumulates binary redactedChunks before making the public opaque
    // thinkingSignature. Typed arrays are not JSON and cannot enter durable state.
    const { index, redactedChunks, ...thinking } = block as ThinkingContent & { index?: unknown; redactedChunks?: unknown };
    return thinking;
  }
  const { index, ...text } = block as TextContent & { index?: unknown };
  return text;
}

export function publicAssistantMessage(message: AssistantMessage): AssistantMessage {
  // Shallow projection, not a deep copy on every token. Durable already copies
  // the latest partial synchronously at its throttled commit boundary. Never
  // delete fields from the provider's mutable object: it still needs its buffers.
  return { ...message, content: message.content.map(publicContent) };
}

function publicEvent(event: AssistantMessageEvent): AssistantMessageEvent {
  if (event.type === "done") return { ...event, message: publicAssistantMessage(event.message) };
  if (event.type === "error") return { ...event, error: publicAssistantMessage(event.error) };
  const partial = publicAssistantMessage(event.partial);
  if (event.type === "toolcall_end") return { ...event, partial, toolCall: publicToolCall(event.toolCall) };
  return { ...event, partial };
}

function publicStream(source: AssistantMessageEventStream): AssistantMessageEventStream {
  let result: Promise<AssistantMessage> | undefined;
  // A lazy facade preserves the provider's queue, cancellation, exceptions and
  // result timing. Do not pump into a second queue or introduce another throttle.
  return new Proxy(source, {
    get(target, key) {
      if (key === Symbol.asyncIterator) return async function* () {
        for await (const event of target) yield publicEvent(event);
      };
      if (key === "result") return () => result ??= target.result().then(publicAssistantMessage);
      const value = Reflect.get(target, key, target);
      return typeof value === "function" ? value.bind(target) : value;
    },
  });
}

/** Keep stock runtime/auth behavior, including methods that use private class state. */
export function durableModels<T extends Models>(models: T): T {
  return new Proxy(models, {
    get(target, key) {
      const value = Reflect.get(target, key, target);
      if (typeof value !== "function") return value;
      if (key === "stream" || key === "streamSimple" || key === "streamDeferred") {
        return (...args: unknown[]) => publicStream(Reflect.apply(value, target, args));
      }
      if (key === "complete" || key === "completeSimple" || key === "fetchDeferred") {
        return (...args: unknown[]) => Reflect.apply(value, target, args).then(publicAssistantMessage);
      }
      return value.bind(target);
    },
  });
}
