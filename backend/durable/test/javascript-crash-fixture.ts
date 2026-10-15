// Offline child runner for the unsafe-JavaScript SQLite crash/reopen test.
import { createModels } from "@earendil-works/pi-ai/models";
import {
  fauxAssistantMessage,
  fauxProvider,
  fauxToolCall,
} from "@earendil-works/pi-ai/providers/faux";
import { run } from "../src/run.ts";
const faux = fauxProvider();
const models = createModels();
models.setProvider(faux.provider);
faux.setResponses([
  ({ messages }) => {
    const result = messages.findLast((m) => m.role === "toolResult");
    if (result)
      return fauxAssistantMessage(
        result.isError
          ? "JavaScript interruption observed"
          : "JavaScript completed",
      );
    return fauxAssistantMessage(
      fauxToolCall(
        "javascript",
        { file: "job.cjs", timeout: 60 },
        { id: "unsafe-script" },
      ),
      { stopReason: "toolUse" },
    );
  },
]);
const model = faux.getModel();
await run(models, [], undefined, {
  state: process.argv[2],
  cwd: process.argv[3],
  key: "javascript-crash",
  input: process.stdin,
  output: (line) => {
    process.stdout.write(line);
  },
  assertOwner: () => {},
  stopInput: () => {},
  initialModel: { provider: model.provider, modelId: model.id },
  onReport: () => {},
});
