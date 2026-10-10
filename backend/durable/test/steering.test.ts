import { expect, test } from "bun:test";
import { BACKGROUND_CONTEXT } from "@earendil-works/chord/context";
import { Type, type Message } from "@earendil-works/pi-ai";
import { createModels } from "@earendil-works/pi-ai/models";
import { fauxAssistantMessage, fauxProvider, fauxToolCall } from "@earendil-works/pi-ai/providers/faux";
import {
  createRegistry, defineExtension, defineTool, Harness, InboxDoc, LiveDoc, MemoryStorage,
} from "@earendil-works/pi-durable";

const context = BACKGROUND_CONTEXT;
// Transaction documents are overlays and must be detached before commit ends.
const snapshot = <T>(value: T): T => JSON.parse(JSON.stringify(value));
const gate = () => {
  let open!: () => void;
  const promise = new Promise<void>((resolve) => { open = resolve; });
  return { promise, open };
};

// As in fixture.ts, use an explicit Models collection containing only the faux
// provider. MemoryStorage is test-local; no credentials, network or sessions.
test("busy steering is queued during tools, then joins the run at the next postTools boundary", async () => {
  const started = [gate(), gate()];
  const release = [gate(), gate()];
  const completed: number[] = [];
  const requests: Message[][] = [];
  const faux = fauxProvider(); // No token delay: gates, not wall-clock sleeps, order the test.
  const models = createModels();
  models.setProvider(faux.provider);
  faux.setResponses([
    (transcript) => {
      requests.push(structuredClone(transcript.messages));
      return fauxAssistantMessage(fauxToolCall("delayed", { round: 1 }, { id: "round-1" }), { stopReason: "toolUse" });
    },
    (transcript) => {
      requests.push(structuredClone(transcript.messages));
      // Keep the same run busy after the first boundary, before any final answer.
      return fauxAssistantMessage(fauxToolCall("delayed", { round: 2 }, { id: "round-2" }), { stopReason: "toolUse" });
    },
    (transcript) => {
      requests.push(structuredClone(transcript.messages));
      return fauxAssistantMessage("Finished with the steering instruction.");
    },
  ]);
  const registry = createRegistry();
  registry.install(defineExtension({ name: "steering-test", tools: [defineTool({
    name: "delayed", description: "A gated offline tool round", replay: "safe",
    parameters: Type.Object({ round: Type.Integer({ minimum: 1, maximum: 2 }) }),
    execute: async ({ round }) => {
      started[round - 1].open();
      await release[round - 1].promise;
      completed.push(round);
      return { content: [{ type: "text", text: `round ${round} complete` }] };
    },
  })] }));
  const harness = await Harness.open(new MemoryStorage(), { models, registry }, context);
  try {
    const model = faux.getModel();
    const root = await harness.root(context, { agent: { model: { provider: model.provider, modelId: model.id } } });
    const initial = await root.submit({ type: "input", content: "Start the work" }, context);
    await started[0].promise;
    const state = () => root.commit(async (tx) => snapshot({
      inbox: await tx.doc(InboxDoc, root.id),
      live: await tx.doc(LiveDoc, root.id),
    }), context);
    const running = await state();
    expect(running.live.run?.inputs).toEqual([initial.id]);
    expect(running.live.tools?.[0].status).toBe("running");

    // Admission must resolve while execute() is still blocked, not wait for the
    // round (or run) to finish. The receipt and durable inbox agree immediately.
    const steer = await root.submit({ type: "input", content: "Use the new plan", whenBusy: "steer" }, context);
    expect((await steer.status(context)).status).toBe("queued");
    const queued = await state();
    expect(queued.inbox.items).toEqual([{ id: steer.id, mode: "steer", content: "Use the new plan" }]);
    expect(queued.live.run?.taskId).toBe(running.live.run?.taskId);
    expect(queued.live.run?.inputs).toEqual([initial.id]);
    expect(completed).toEqual([]);
    expect(faux.state.callCount).toBe(1);
    expect((await root.context(context)).messages.some((message) => message.role === "user" && message.content === "Use the new plan")).toBe(false);

    release[0].open();
    await started[1].promise;
    expect(completed).toEqual([1]);
    expect(faux.state.callCount).toBe(2);
    // The very next request sees the steer *after* the finished round's result.
    // No final assistant answer or new agent run was needed to consume it.
    const nextRequest = requests[1].filter((message) => message.role !== "system");
    expect(nextRequest.map((message) => message.role)).toEqual(["user", "assistant", "toolResult", "user"]);
    expect(nextRequest[2]).toMatchObject({ role: "toolResult", toolCallId: "round-1", content: [{ type: "text", text: "round 1 complete" }] });
    expect(nextRequest[3]).toMatchObject({ role: "user", content: "Use the new plan" });
    expect((await steer.status(context)).status).toBe("placed");
    expect((await initial.status(context)).status).toBe("placed");
    const continued = await state();
    expect(continued.inbox.items).toEqual([]);
    expect(continued.live.run?.inputs).toEqual([initial.id, steer.id]);
    expect(continued.live.tools?.[0]).toMatchObject({ callId: "round-2", status: "running" });

    release[1].open();
    const [answer, steeredAnswer] = await Promise.all([initial.wait(context), steer.wait(context)]);
    expect(answer).toMatchObject({ type: "input", status: "done" });
    expect(steeredAnswer).toMatchObject({ type: "input", status: "done" });
    if (answer.status === "done" && answer.type === "input" && steeredAnswer.status === "done" && steeredAnswer.type === "input") {
      expect(steeredAnswer.answer).toBe(answer.answer);
    }
    await root.waitForIdle(context);
    expect(faux.state.callCount).toBe(3);
    expect((await state()).live.run).toBeUndefined();
  } finally {
    // Also unblock tools on assertion failures so closing cannot strand a gate.
    release.forEach((item) => item.open());
    await harness.close(context);
  }
});
