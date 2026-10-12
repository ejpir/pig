/** Adapt only generation preparation's settings; preserve the pinned task's
 * identity, checkpoints, execution phases and recovery behavior. The upstream
 * settings getter is harness-wide, but percentage triggers need the actual
 * generation model's window, not the root model's window.
 */
import { GenerationTask, type RegistryReader, type RegistrySnapshot } from "@earendil-works/pi-durable";
import { compactionPolicy } from "./session-settings.ts";

type Metadata = { autoCompaction: boolean; compactionThresholdPercent?: number };

export function compactionRegistry(registry: RegistryReader, metadata: () => Metadata): RegistryReader {
  const prepare = GenerationTask.definition.phases.prepare;
  const generation: typeof GenerationTask = {
    definition: {
      ...GenerationTask.definition,
      phases: {
        ...GenerationTask.definition.phases,
        prepare: async (task, runtime, context) => {
          const agent = await runtime.agent(context);
          const model = agent.model && runtime.models.getModel(agent.model.provider, agent.model.modelId);
          const scoped = new Proxy(runtime, {
            get(target, key) {
              if (key === "settings") return { ...target.settings, compaction: compactionPolicy(metadata(), model?.contextWindow) };
              return Reflect.get(target, key, target);
            },
          });
          return prepare(task, scoped, context);
        },
      },
    },
  };
  const snapshots = new WeakMap<RegistrySnapshot, RegistrySnapshot>();
  return {
    subscribe: (listener) => registry.subscribe(listener),
    snapshot: () => {
      const snapshot = registry.snapshot();
      let adapted = snapshots.get(snapshot);
      if (!adapted) {
        adapted = new Proxy(snapshot, {
          get(target, key) {
            if (key === "task") return (name: string) => name === GenerationTask.definition.name ? generation : target.task(name);
            if (key === "tasks") return () => target.tasks().map((task) => task.definition.name === GenerationTask.definition.name ? generation : task);
            const value = Reflect.get(target, key, target);
            return typeof value === "function" ? value.bind(target) : value;
          },
        });
        snapshots.set(snapshot, adapted);
      }
      return adapted;
    },
  };
}
