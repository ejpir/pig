// Offline test-only SDK integration, imported by the separate Android JNI probe.
import { writeFile } from "node:fs/promises";
import { join } from "node:path";
import { BACKGROUND_CONTEXT as context } from "@earendil-works/chord/context";
import { createModels } from "@earendil-works/pi-ai/models";
import {
  fauxProvider,
  fauxAssistantMessage,
  fauxToolCall,
} from "@earendil-works/pi-ai/providers/faux";
import {
  createRegistry,
  Harness,
  MemoryStorage,
} from "@earendil-works/pi-durable";
import { NodeExecutionEnv } from "@earendil-works/pi-durable/env/node";
import { JavascriptTools } from "../src/javascript.ts";

export async function javascriptSdkProbe(cwd: string) {
  await writeFile(
    join(cwd, "sdk-script.cjs"),
    "console.log(require('node:crypto').createHash('sha256').update('sdk-worker').digest('hex'));console.error('sdk-stderr');",
  );
  const models = createModels(),
    faux = fauxProvider();
  models.setProvider(faux.provider);
  faux.setResponses([
    fauxAssistantMessage(
      fauxToolCall("javascript", { file: "sdk-script.cjs" }, { id: "script" }),
      { stopReason: "toolUse" },
    ),
    fauxAssistantMessage("done"),
  ]);
  const registry = createRegistry();
  registry.install(JavascriptTools);
  const harness = await Harness.open(
    new MemoryStorage(),
    {
      models,
      registry,
      env: () => new NodeExecutionEnv({ cwd, shellPath: "/system/bin/sh" }),
    },
    context,
  );
  try {
    const model = faux.getModel();
    const root = await harness.root(context, {
      agent: { model: { provider: model.provider, modelId: model.id } },
    });
    const submission = await root.submit(
      { type: "input", content: "offline JavaScript tool" },
      context,
    );
    await submission.wait(context);
    await root.waitForIdle(context);
    const result = (await root.context(context)).messages.find(
      (message) => message.role === "toolResult",
    );
    if (!result || result.role !== "toolResult" || result.isError)
      throw Error("SDK JavaScript tool did not succeed");
    const output = result.content
      .filter((block) => block.type === "text")
      .map((block) => block.text)
      .join("");
    if (
      !output.includes("sdk-stderr") ||
      !output.includes("[JavaScript: exit 0]")
    )
      throw Error("SDK did not retain worker output/status");
    return { passed: true, output, details: result.details };
  } finally {
    await harness.close(context);
  }
}
