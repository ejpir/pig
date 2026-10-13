# Native JavaScript visual apps

`pi_visual` is a portable native UI/shape vocabulary with an embedded **QuickJS-NG** JavaScript interpreter (rquickjs 0.14). There is no DOM, HTML/WebView, Node process, per-visualization Rust compilation, or registered filesystem/network/process/agent capability.

Opted-in assistant replies contain one explicitly closed, top-level fence:

````text
```javascript pi-visual
app({title:'Counter',state:{count:0},view:ui.column([
  ui.text(s=>`Count ${s.count}`,{style:'display'}),
  ui.button('Add one',s=>{s.count++;},{tone:'accent'})
])});
```
````

Ordinary JavaScript fences and partial, quoted or nested fences stay inert. `app` registers one static native UI. No escaped JSON envelope is needed.

## API

- `ui.column(children,{gap:8,width:null})`, `ui.row(children,{gap:8})`
- `ui.text(literalOrGetter,{style:'muted'|'display'|'expression'|'heading'|'history'})`
- `ui.button(label,onPress,{tone:'normal'|'accent'|'utility'|'operator'})`
- `ui.slider(label,field,{min,max,step:1,onChange})`
- `ui.canvas({label,height:240,viewBox:[0,0,100,100],draw(g,s),onPointer(s,x,y)})`
- Optional `animation:{running:'playing',tick(s,dt),fps}`. Omitted fps follows the actual display; an explicit 1..240 cap is honored. `dt` is elapsed seconds capped at 0.1. Host visibility/focus/sheet/reduced-motion policy suspends animation.

Event handlers mutate a draft, with atomic host validation/commit. Slider candidates are clamped/quantized before onChange. Cancellation restores the whole pre-gesture state; local persistence remains host-controlled, on user interactions rather than frames. Handler return values are ignored; any guest Promise creation or pending jobs are rejected using a native promise hook, even when the promise is unreturned or has a changed prototype.

Text getters return bounded strings from a frozen state snapshot. They are batched and cached by state; derived labels never enter persisted state. Drawing receives another frozen snapshot and records native shapes:

```js
g.line(x1,y1,x2,y2,{color:'#70B8FF',width:1.5});
g.circle(x,y,r,{color:'#70B8FF',width:1.5,fill:'#70B8FF'});
g.rect(x,y,w,h,{color:'#70B8FF',width:1.5,fill:'#70B8FF'});
g.polyline([[x,y],...],{color:'#70B8FF',width:1.5});
```

Geometry is uniformly fitted to/clipped by viewBox; stroke widths are dp. Polylines automatically segment into overlapping chunks. Limits: four canvases, 64 shapes/scene, 2048 total vertices (circle=64), 64 vertices/chunk, finite coordinates abs<=1e6, nonnegative sizes/radii, widths0.5..8, validated #RRGGBB/#RRGGBBAA colors. Budget the completed trace, not just startup. Native images/text/gradients/transforms/arbitrary filled paths are not part of this initial drawing API.

## Execution boundaries

Rust owns definition, fixed flat state (64 fields; finite Number/boolean/string<=256 UTF-8 bytes), and validated caches. **Every uncached draw, text batch and event gets a fresh bounded VM/context.** Immutable, process-local QuickJS bytecode is cached (at most 512KiB/app plus one shared bootstrap); bootstrap and source initialization are still executed afresh. No bytecode is accepted from guests/files, persisted, or shared across builds/architectures. Mutable closures/globals/prototypes/jobs cannot survive a call or escape gesture rollback. App registration must remain deterministic. All changing data belongs in state. Unchanged numeric state preserves scene caches; faults retain the last validated scene with an explicit host error.

Resource policy: 64KiB source, 32KiB serialized definition, 128 UI nodes/depth12, 16MiB guest heap, 256KiB guest stack, 50ms initial registration and cooperative20ms invocation covering VM creation/source evaluation/callback/extraction. Native array methods are guarded at 8192 elements before entering C (including generic sparse-array calls), because native loops do not all poll VM interrupts. QuickJS interrupt polls are not instruction counts. Guest limits do not bound all process RSS and are not OS/process isolation or a hard real-time guarantee.

Optional aggregate CPU profiling measures callback/drawing-VM/text-VM/decode, input cadence/coalescing wait, app-root layout+prepaint, app-root painting/path cache reuse, and native acquire/encode+submit/present-call durations. None establishes GPU execution time or actual presented FPS. No coordinate/source/state payload is logged by these counters; rings are bounded.

## Rhai migration

Rhai is no longer a dependency or executable format. Historical JSON/Rhai replies are not translated automatically: source inspection remains available, and unsupported initialization is not admitted as an executable visual. Saved state/source files are **not deleted**. Regenerate with `@Visualize` for the JavaScript format; changed source has a separate local state identity. Historical admitted prompt guides remain recognized for label stripping/retry identity, not for execution.

Samples are in `pi_android/assets/samples/*.pi-visual.js`. Tests cover transactions, schema/resource guards, synchronous-only behavior, absent host capabilities, callback isolation, cached/static drawing, native gestures and full-trace completion. These tests do not prove arbitrary generated-code safety or measured physical-phone120FPS.
