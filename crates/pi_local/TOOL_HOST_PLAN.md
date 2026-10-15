# Unfinished stock-tool integration

Local must implement the same execution contract as remote Pi. No reduced-chat UI,
provider-specific destination, fake workspace, fake Bash or companion navigation.
Tools are not yet connected to subscription execution. Current `js/chat.js` still
uses `tools: []`, a one-request budget, and unconditional turn ending; therefore the
backend is not complete.

## Required implementation

- Use unchanged `@earendil-works/pi-coding-agent@1.2.0` public tool factories with
  platform operations. Keep npm algorithms untouched and dependencies locked.
- Supply confined filesystem operations mapped from a virtual workspace to native
  descriptor-relative capabilities. Credentials, private app data and arbitrary
  native paths are never model-readable. Preserve upstream read/edit/write behavior,
  image handling, mutation serialization and truncation; do not substitute fixture
  implementations.
- Complete the upstream Agent tool loop, bounded multi-request execution, canonical
  intermediate assistants/tool results, normalized tool events, and cancellation
  with worker/process teardown. Do not settle a run at `message_end` or the first
  `turn_end`. Persist successful canonical tool sequences separately from UI state.
- Real Bash requires a **credential-free UID boundary**. An app-UID subprocess,
  different cwd/environment or encrypted vault is not isolation. Android's
  `isolatedProcess` cannot traverse app-private workspace paths even through a
  directory descriptor; a conventional full workspace needs a separate secret-free
  tool UID/package or a substantial broker/virtual filesystem. No companion package
  has been approved or added. `/system/bin/sh` is mksh, not Bash.
- Tool execution must have wall/output/process/memory limits, descendant cleanup,
  concurrent pipe draining, death/cancellation handling, and no automatic replay
  after uncertain effects. Package executable code read-only; never execute a
  downloaded binary from writable app data. Shipping Bash also needs source/license
  compliance.

## Adapter findings

Read/write/edit/ls/find expose useful operation injection. Injection alone is not
sufficient: read probes fs access, edit/write queues use realpath, and tool renderers
import TUI/theme/config. Image processing statically imports Photon/worker machinery.
Grep still directly spawns rg; shell uses environment/temp output adapters.
The root package export is public; internal deep imports are not a substitute for
adapting its supported surface. A browser-bundle probe of the public exports failed
on unadapted Node builtins; that probe is **not** a working tool implementation.

The stock system-prompt builder is not publicly exported. Generate an effective
prompt through a non-inferencing public AgentSession/resource loader at build time,
or adapt supported public runtime APIs; do not copy internal prompt algorithms.

## Acceptance gates

1. Common UI/control tests: one Local backend, shared model picker, account lifecycle,
   retained sessions/drafts, no added navigation, no automatic inference.
2. Real upstream tool round trips: read/write/edit/list/find and full canonical
   follow-up context, bounded errors/cancellation and presentation.
3. Bash workspace semantics and attempts to access vault/Keystore/app-private paths
   fail across the genuine isolation boundary, not merely a path allowlist.
4. All stock-tool/platform import branches fail closed; no hidden credential or
   unrestricted process capability reaches model code or generated Visualize.
5. Host, emulator and physical-architecture artifacts rebuilt; synthetic evidence
   remains explicitly distinct from live subscription validation.
