import { desktopModels } from "./models.ts";
import { run } from "./run.ts";

// Credentials/config are read on the SSH host; they are never copied from the desktop.
await run(await desktopModels());
