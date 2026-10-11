import { expect, test } from "bun:test";
import { BACKGROUND_CONTEXT } from "@earendil-works/chord/context";
import { createModels } from "@earendil-works/pi-ai/models";
import { createRegistry, Harness, MemoryStorage, watchEvents } from "@earendil-works/pi-durable";
import { eventStreamFailure } from "../src/run.ts";

const context = BACKGROUND_CONTEXT;

async function opened() {
  const harness = await Harness.open(new MemoryStorage(), { models: createModels(), registry: createRegistry() }, context);
  const root = await harness.root(context);
  return { harness, root };
}

test("an event listener failure is a fatal stream failure", async () => {
  const { harness, root } = await opened();
  const events = await watchEvents(harness, root.id, context);
  const listenerError = new Error("wire delivery failed");
  try {
    events.start(async () => { throw listenerError; });
    const failed = eventStreamFailure(events.closed);
    await root.submit({ type: "write", entry: { kind: "test.event" } }, context);
    await expect(failed).rejects.toThrow("Durable event stream listener failed: wire delivery failed");
    expect((await events.closed).reason).toBe("listener_error");
  } finally {
    await events.stop();
    await harness.close(context);
  }
});

test("an unexpected session closure is a fatal stream failure", async () => {
  const { harness, root } = await opened();
  const events = await watchEvents(harness, root.id, context);
  events.start(async () => {});
  const failed = eventStreamFailure(events.closed);
  await harness.close(context);
  await expect(failed).rejects.toThrow("Durable event stream closed unexpectedly: session_closed");
});
