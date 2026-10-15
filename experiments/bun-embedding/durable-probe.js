// Trusted offline probe: real unchanged Durable SDK/tools, synthetic model only.
// No credential imports, provider network requests, mocks of fs/SQLite or SDK patches.
import { mkdir, mkdtemp, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { BACKGROUND_CONTEXT as context } from "@earendil-works/chord/context";
import { createModels } from "@earendil-works/pi-ai/models";
import {
  fauxProvider,
  fauxAssistantMessage,
  fauxToolCall,
} from "@earendil-works/pi-ai/providers/faux";
import {
  Harness,
  createRegistry,
  AssistantEntry,
} from "@earendil-works/pi-durable";
import { CodingTools } from "@earendil-works/pi-durable/tools";
import { NodeExecutionEnv } from "@earendil-works/pi-durable/env/node";
import { SqliteStorage } from "@earendil-works/pi-durable/storage/sqlite";
import { openNodeSqliteDatabase } from "@earendil-works/pi-durable/storage/sqlite/node";

function check(value, message) {
  if (!value) throw new Error(message);
}
function text(message) {
  return typeof message.content === "string"
    ? message.content
    : message.content
        .filter((block) => block.type === "text")
        .map((block) => block.text)
        .join("\n");
}
const requestId = "offline-tools-v1";
const draft = {
  type: "input",
  content: "Run the trusted offline tool probe.",
  requestId,
  whenBusy: "reject",
};
const script = [
  ["write", { path: "probe.txt", content: "before\n" }, null],
  ["read", { path: "probe.txt" }, "before\n"],
  [
    "edit",
    { path: "probe.txt", edits: [{ oldText: "before", newText: "after" }] },
    null,
  ],
  [
    "bash",
    {
      command:
        'IFS= read -r line < probe.txt && [ "$line" = after ] && printf \'shell-ok:%s\\n\' "$line"',
      timeout: 5,
    },
    "shell-ok:after\n",
  ],
  ["read", { path: "probe.txt" }, "after\n"],
];

export async function runProbe({
  root: parent,
  phase,
  shellPath = "/system/bin/sh",
}) {
  check(phase === "create" || phase === "recover", "Unknown probe phase");
  await mkdir(parent, { recursive: true });
  const locator = join(parent, "last-run.json");
  const ids =
    phase === "create"
      ? {
          directory: await mkdtemp(join(parent, "run-")),
        }
      : JSON.parse(await readFile(locator, "utf8"));
  ids.databasePath ??= join(ids.directory, "session.sqlite");
  // Keep output spills and HOME within this credential-free test package too.
  process.env.HOME = ids.directory;
  process.env.TMPDIR = ids.directory;
  const faux = fauxProvider({ tokensPerSecond: 100000 });
  const models = createModels();
  models.setProvider(faux.provider);
  const environments = new Set();
  const registry = createRegistry();
  registry.install(CodingTools);
  const options = {
    models,
    registry,
    settings: { compaction: { enabled: false }, retry: { enabled: false } },
    env: ({ cwd }) => {
      const env = new NodeExecutionEnv({
        cwd: cwd ?? ids.directory,
        shellPath,
      });
      environments.add(env);
      return env;
    },
    onReport: (error) => {
      globalThis.__durableLastReport = String(error);
    },
  };
  async function open() {
    const database = await openNodeSqliteDatabase(ids.databasePath);
    try {
      await database.exec("PRAGMA synchronous = FULL");
      const storage = await SqliteStorage.open(database);
      return await Harness.open(storage, options, context);
    } catch (error) {
      await database.close().catch(() => {});
      throw error;
    }
  }
  let harness;
  async function close() {
    const current = harness;
    harness = undefined;
    try {
      await current?.close(context);
    } finally {
      await Promise.all([...environments].map((env) => env.cleanup(context)));
      environments.clear();
    }
  }
  function respond({ messages }) {
    check(
      phase === "create",
      "Completed prompt was replayed after a process restart",
    );
    for (const [index, [name, args, expected]] of script.entries()) {
      const id = `probe-${index}`;
      const result = messages.find(
        (message) => message.role === "toolResult" && message.toolCallId === id,
      );
      if (!result)
        return fauxAssistantMessage(fauxToolCall(name, args, { id }), {
          stopReason: "toolUse",
        });
      check(!result.isError, `${name} failed: ${text(result)}`);
      if (expected !== null)
        check(
          text(result) === expected,
          `${name} output mismatch: ${JSON.stringify(text(result))}`,
        );
    }
    return fauxAssistantMessage("offline-sdk-ok");
  }
  faux.setResponses(Array(script.length + 1).fill(respond));
  try {
    harness = await open();
    if (phase === "create") {
      const model = faux.getModel();
      const conversation = await harness.root(context, {
        agent: {
          cwd: ids.directory,
          model: { provider: model.provider, modelId: model.id },
        },
      });
      ids.conversationId = conversation.id;
      const submission = await conversation.submit(draft, context);
      ids.submissionId = submission.id;
      const done = await submission.wait(context);
      check(
        done.type === "input" && done.status === "done",
        `Prompt failed: ${JSON.stringify(done)}`,
      );
      await conversation.waitForIdle(context);
      check(
        faux.state.callCount === script.length + 1,
        "Unexpected provider call count",
      );
      ids.entryId = done.entry;
      ids.answerId = done.answer;
      await close();
      await writeFile(locator, JSON.stringify(ids) + "\n", { mode: 0o600 });
      harness = await open();
    }
    const conversation = await harness.conversation(
      ids.conversationId,
      context,
    );
    const submission = await harness.submission(ids.submissionId, context);
    check(
      conversation && submission,
      "Persisted conversation/submission disappeared",
    );
    const receipt = await submission.status(context);
    check(
      receipt.type === "input" &&
        receipt.status === "done" &&
        receipt.entry === ids.entryId &&
        receipt.answer === ids.answerId,
      "Committed completion receipt changed after SQLite reopen",
    );
    const answer = await conversation.commit(
      (tx) => tx.entry(AssistantEntry, receipt.answer),
      context,
    );
    check(
      answer?.model?.[0] && text(answer.model[0]) === "offline-sdk-ok",
      "Persisted assistant answer missing",
    );
    const history = await conversation.context(context);
    const results = history.messages.filter(
      (message) => message.role === "toolResult",
    );
    check(
      results.length === script.length &&
        results.every((result) => !result.isError),
      "Committed tool results missing",
    );
    const content = await readFile(join(ids.directory, "probe.txt"), "utf8");
    check(content === "after\n", "Real write/edit result not retained");
    const callsBefore = faux.state.callCount;
    harness.resume();
    const duplicate = await conversation.submit(draft, context);
    check(
      duplicate.id === ids.submissionId,
      "Completed request deduplication failed",
    );
    await conversation.waitForIdle(context);
    check(
      faux.state.callCount === callsBefore,
      "Deduplicated prompt unexpectedly performed inference",
    );
    return {
      passed: true,
      phase,
      pid: process.pid,
      bun: Bun.version,
      durableVersion: "1.2.0",
      conversationId: ids.conversationId,
      submissionId: ids.submissionId,
      databasePath: ids.databasePath,
      storage: "public SQLite node adapter (node:sqlite), synchronous=FULL",
      sqliteReopened: true,
      completedReceiptRestored: true,
      answerRestored: true,
      toolResultsRestored: true,
      completedRequestDeduplicated: true,
      tools: results.map((result) => result.toolName),
      fileContent: content,
      shellPath,
      commandOutput: results
        .filter((result) => result.toolName === "bash")
        .map(text),
      syntheticModelCalls: faux.state.callCount,
      unchangedToolAdapter: "NodeExecutionEnv",
      noRealProviders: true,
      notASandbox: true,
    };
  } finally {
    await close();
  }
}
