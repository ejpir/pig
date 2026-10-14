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

## Shared desktop and Android host

Both frontends use [`pi_visual_ui`](../pi_visual_ui/README.md) for GPUI rendering,
controls, input, frame scheduling and local state. This crate owns the portable
runtime, SVG frontend, shared prompt admission helpers and samples. There is no
Android application dependency in the desktop host.

Desktop requires completed replies with unambiguous durable user/assistant history
identities and explicit original-prose opt-in. Attachment bodies and generated
mention labels cannot supply that consent; streaming or unmatched replies stay
Markdown. Historical Android contracts remain recognized. Hidden, blurred and
unmounted cards stop animation and cancel transient input, including text-only apps.

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

Geometry is uniformly fitted to/clipped by viewBox; stroke widths are dp. Polylines automatically segment into overlapping chunks. Limits: four canvases, 64 shapes/scene, 2048 total vertices (circle=64), 64 vertices/chunk, finite coordinates abs<=1e6, nonnegative sizes/radii, widths0.5..8, validated #RRGGBB/#RRGGBBAA colors. Budget the completed trace, not just startup. These legacy primitives remain compatible; additional paths, gradients, transforms, canvas text and layers are described below. Images remain unsupported.

## Execution boundaries

Rust owns definition, fixed flat state (64 fields; finite Number/boolean/string<=256 UTF-8 bytes), and validated caches. **Every uncached draw, text batch and event gets a fresh bounded VM/context.** Immutable, process-local QuickJS bytecode is cached (at most 512KiB/app plus one shared bootstrap); bootstrap and source initialization are still executed afresh. No bytecode is accepted from guests/files, persisted, or shared across builds/architectures. Mutable closures/globals/prototypes/jobs cannot survive a call or escape gesture rollback. App registration must remain deterministic. All changing data belongs in state. Unchanged numeric state preserves scene caches; faults retain the last validated scene with an explicit host error.

Resource policy: 64KiB source, 32KiB serialized definition, 128 UI nodes/depth12, 16MiB guest heap, 256KiB guest stack, 50ms initial registration and cooperative20ms invocation covering VM creation/source evaluation/callback/extraction. Native array methods are guarded at 8192 elements before entering C (including generic sparse-array calls), because native loops do not all poll VM interrupts. QuickJS interrupt polls are not instruction counts. Guest limits do not bound all process RSS and are not OS/process isolation or a hard real-time guarantee.

Optional aggregate CPU profiling measures callback/drawing-VM/text-VM/decode, input cadence/coalescing wait, app-root layout+prepaint, app-root painting/path cache reuse, and native acquire/encode+submit/present-call durations. None establishes GPU execution time or actual presented FPS. No coordinate/source/state payload is logged by these counters; rings are bounded.

## Rhai migration

Rhai is no longer a dependency or executable format. Historical JSON/Rhai replies are not translated automatically: source inspection remains available, and unsupported initialization is not admitted as an executable visual. Saved state/source files are **not deleted**. Regenerate with `@Visualize` for the JavaScript format; changed source has a separate local state identity. Historical admitted prompt guides remain recognized for label stripping/retry identity, not for execution.

Samples are in `pi_visual/assets/samples/*.pi-visual.js`. Tests cover transactions, schema/resource guards, synchronous-only behavior, absent host capabilities, callback isolation, cached/static drawing, native gestures and full-trace completion. These tests do not prove arbitrary generated-code safety or measured physical-phone120FPS.

## Rich scenes (additive JavaScript v3 API)

```js
const logo = ui.path([
  ['M', 0, 0], ['Q', 20, 0, 20, 20],
  ['C', 20, 30, 0, 30, 0, 20], ['Z']
]);
// Inside draw:
g.group({translate: [50, 50], rotate: 30, scale: 1.5}, g => {
  g.path(logo, {fill: ui.linearGradient(90, '#70B8FF', '#B0EFFF'),
    color: '#FFFFFF', width: 1.5, fillRule: 'nonzero'});
});
g.text('Score 42', 5, 5, {size: 18, color: '#FFFFFF', font: 'mono'});
```

- `ui.path` makes an immutable, reusable path: `M`, `L`, `Q`, `C`, `Z` commands. Start each contour with `M`, including after `Z`. Fill rules: `nonzero` (default), `evenodd` (holes). Fill/stroke are independently optional; at least one is required.
- Maximum 256 commands/path; curves still cost 32 **source** vertices (circles64) against the existing 2048-vertex scene budget. Native rendering now chooses density/zoom-aware subdivision, described below.
- `ui.linearGradient(angle, from, to)` supplies two-stop paint for geometry, including alpha colors. Angles are degrees clockwise from top, relative to final painted bounds. No radial/multi-stop gradients or custom shaders.
- Groups compose local scale, rotation (degrees), translation; up to eight nested groups. Groups count toward the 64-shape scene budget. Group transforms affect strokes too; viewBox fitting alone preserves dp stroke widths.
- Canvas text: single line, `align:'left'|'center'|'right'`, `font:'sans'|'mono'`; position anchors its top edge. Font size is dp, 4–128 before group scaling, 4–256 after. Solid paint only. Translation/positive uniform scale supported; rotated, reflected, or nonuniformly scaled text is rejected. Each line <=2048 UTF-8 bytes; total <=2048 characters/scene.

### Reusable drawing layers

Use `layers` **instead of** `draw` on a canvas:

```js
ui.canvas({label: 'Scene', layers: [
  {deps: [], draw(g) { /* static scenery */ }},
  {deps: ['x'], draw(g, s) { /* moving character */ }}
]});
```

Up to eight ordered layers. `deps:[]` is static; omitted `deps` means all state. Dependency-scoped callbacks receive **only** their declared fields. Missing layers each use a fresh isolated VM (sharing one total 20ms invocation budget); validated results commit atomically, and all layers share the canvas's shape/vertex/text budgets. Failed draws retain the last validated complete scene. Cached layers contain native drawing data, never live JS closures.

The Android adapter reuses tessellated geometry across matching instances, placement changes and paint changes; only the current frame's resources are retained. ViewBox scale changes rebuild geometry. This is a CPU geometry/display-list cache, **not** an offscreen bitmap or persistent GPU layer cache. Text shaping is cached separately. These changes make no measured FPS guarantee.

### Held input, keyboard and multitouch

`ui.canvas({onInput(s,event,input){...}, ...})` opts into phased input instead of legacy `onPointer`:

- Pointer: `{kind:'pointer',id,phase:'down'|'move'|'up'|'cancel',x,y}`. Up arrives even without motion. Up to ten contacts; moves coalesce per contact/frame.
- Keyboard: `{kind:'key',key,down,repeat}`. Focus by tapping or Tab; common names include `left`, `right`, `up`, `down`, `space`, `enter`, lowercase letters. Tab/Escape/Ctrl/platform shortcuts remain outside the canvas handler.
- `{kind:'blur'}` releases keyboard state; `{kind:'cancel'}` releases the surface's inputs. Multiple contacts are independent; cancel does not undo unrelated gameplay or another contact's transactions.
- `ui.holdButton(label,name,{tone})` is a native held control, not a click callback. Poll `input.buttons.includes(name)` during animation.

Both `onInput(s,event,input)` and `tick(s,dt,input)` receive a frozen snapshot:
`{pointers:[{target,id,x,y}],keys:[],buttons:[]}`. `target` is the canvas's `inputId` (or generated callback ID). Held input is host-owned, bounded, and **never persisted**. Focus loss clears keys; navigation/background/unmount clears held input. Guest faults roll back guest state but do not prevent physical release. Legacy click/buttons/sliders/onPointer retain their existing semantics.

Multitouch routing lives entirely in `gpui_android::raw_touch`: explicitly opted-in surfaces claim raw streams using a scoped adapter envelope around GPUI's existing drag-event transport. Unclaimed contacts continue through GPUI's recognizer unchanged. No sibling Zed modifications are required.

Images/sprites, gamepads, arbitrary blur/glow, advanced blending and transformed mixed-content compositing remain out of scope. See `pi_visual/assets/samples/rich-scenes.pi-visual.js`.


### HiDPI rendering

Android already draws paths and glyphs at the render surface's display density. Vector subdivision now also accounts for device pixel ratio, viewBox fitting and the maximum stretching of composed group transforms (including nonuniform scale, shear and reflection). Translation and rotation alone do not demand finer geometry. Logical layout, text sizes, dp stroke widths, and input coordinates are unchanged; increasing viewBox numbers is not a resolution setting.

The renderer targets **0.25 physical-pixel curve approximation error**, using power-of-two levels up to **128 segments per Bézier** and **256 per circle**. Tiny/straight filled curves use fewer segments; stroked curves retain a32-segment floor and account for thick outlines. Circle quality also accounts for dp stroke thickness; the native stroke builder preserves planned close-together vertices instead of merging them with its default tolerance. This is a best-effort target: extreme zoom reaches the subdivision cap, and floating-point evaluation/placement error is not covered by the analytic approximation bound. Native text continues to use GPUI's density-aware glyph rasterization.

Subdivision choices are part of geometry cache identity: crossing a density/zoom quality level rebuilds the affected mesh, while unchanged levels reuse it. Fit-scale changes still invalidate geometry stored in fitted logical coordinates. Caches retain only the current scene's resources; there are no persistent caches for every visited zoom level.

Guest limits remain unchanged, so existing apps do not consume extra script budget on denser displays. Native work has separate per-scene limits: **8192 contour points** and **131072 expanded mesh vertices**, counting every instance and both fill/stroke meshes even on cache hits. Contour plans are bounded before tessellation. The repository-local Lyon output adapter now enforces remaining vertex/triangle allowances during construction, before GPUI triangle expansion, caching or emission. Internal tessellator scratch allocation and wall-clock time are still not preempted; these are bounded-input/output checks, not OS isolation. Native tessellation failures remain visible on the canvas. No shader or sibling Zed changes are required.


### Native SVG subset

SVG is imported as validated vector commands, **not an image, browser or asset loader**:

```javascript
const check = ui.path('M3 12 l7 7 L21 4');
const icon = ui.svg(`<svg viewBox="0 0 24 24">
  <path d="M3 12 l7 7 L21 4" fill="none" stroke="#70B8FF" stroke-width="2"/>
</svg>`);
// Inside draw(g,s):
g.path(check, {color: '#70B8FF', width: 2}); // legacy dp stroke width
g.svg(icon, {x: 30, y: 10, width: 48, height: 48}); // SVG source-unit strokes
```

Define reusable assets outside draw. `ui.svg` returns immutable `width`, `height`, `viewBox` metadata and an invocation-local import identity; `g.svg` requires this imported value, not an arbitrary object or URL. Width/height options default to intrinsic dimensions (SVG dimensions, otherwise viewBox dimensions, otherwise300×150). Placement dimensions must be positive. Host-owned pure parse results are cached independently of JS closures/state; changing translation/color does not invalidate native geometry. SVG gradient coordinates follow the full transform rather than the bounds-relative paint convention of `ui.linearGradient`.

**Supported:**
- Path strings: absolute/relative `M L H V C S Q T A Z`, exponent numbers, implicit repetitions, smooth-control reflection and drawing after close. Array-form `ui.path` remains unchanged.
- One `svg` root, `g`, `path`, `rect` (including `rx`/`ry`), `circle`, `ellipse`, `line`, `polyline`, `polygon`. `title`/`desc` are non-rendered metadata.
- `viewBox`, numeric/px sizes, default `xMidYMid meet` or `preserveAspectRatio="none"`. SVG transforms: matrix, translate, scale, rotate (including a center), skewX/Y.
- Inherited presentation attributes: fill/stroke (default black/none), stroke-width, fill-rule, color/currentColor, fill-opacity/stroke-opacity. Hex, named and CSS-style color values supported by the bounded color parser. No CSS rules or style attributes. Butt stroke caps, miter joins with limit4 only. Zero stroke width disables stroke; SVG widths scale with both viewBox fitting and groups, unlike legacy dp paths.
- `defs/linearGradient` with a unique id and **exactly two stops at0 and1**; stop-color and stop-opacity; local `url(#id)` references; pad spread; objectBoundingBox (default) or userSpaceOnUse; gradientTransform. Zero-area geometry ignores object-bbox paint servers, including horizontal/vertical stroked lines; use a user-space gradient there. Coincident gradient endpoints use the final stop. Singular/ill-conditioned gradient transforms fail explicitly.

**Not supported:** scripts/events, external resources, embedded/raster images, text/fonts, CSS/style, use/symbol, group opacity/compositing, filters, masks, clipping paths, radial/multi-stop/repeating gradients, dashes, round/bevel joins or round/square caps, nested SVG, alternative viewport alignment/slice. Unsupported tags/attributes fail even in defs or invisible content; no resource is fetched. SVG viewport mapping does **not** create a separate clip: only the enclosing canvas clips, so keep artwork within its viewBox (`overflow="hidden"` is rejected). Percentages are supported only for gradient coordinates, offsets and opacity; geometry percentages and other length units are rejected. DTD/custom entities are forbidden; predefined/numeric XML escapes are allowed.

**Limits:**16KiB UTF-8 per XML/path string,512 XML nodes,16 gradients,32 transform tokens/list;32 native parse calls and64KiB total submitted strings per isolated invocation, including registration, failed calls and cache hits. The pure parse cache has at most8 entries and512KiB aggregate source/output data. Call quotas and deadlines are checked around native parsing; native code itself is not preempted. Compiled JSON is limited to128KiB.

SVG wrappers, groups and every lowered path command count toward the same64-shape,2048-weighted-vertex,depth8 scene budget **per drawn instance**, including cached imports and shared layers. Paths have at most256 input and256 lowered commands. Elliptical arcs are approximated with at most8 cubic pieces, each spanning at most45 degrees, followed by bounded HiDPI tessellation; an ellipse generally costs258 source vertices. Extreme zoom remains best-effort. No saved-state schema or legacy drawing/input behavior changes are required. Demo: `crates/pi_visual/assets/samples/svg-postcard.pi-visual.js`, preview `visual-svg` / `visual-svg-evening`.
