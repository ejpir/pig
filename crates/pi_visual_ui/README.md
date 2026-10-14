# Shared native Visualize host

Android and desktop use this GPUI host rather than separate renderers. `pi_visual`
owns the JavaScript runtime, SVG compiler, scene/input contracts, prompt admission
helpers and samples. This crate owns cards, controls, phased input, native painting,
HiDPI tessellation, mesh budgets, profiling, local state and frame scheduling.

A frontend implements `Host`: route/availability, interaction persistence policy,
transcript remeasurement, focus and optional source presentation. It admits complete
assistant replies and supplies immutable keys; `HostExt` renders and dispatches them.
Desktop uses window-native mouse/keyboard events. The optional `android` feature
adds only the platform raw-touch/tracing adapter, not the Android application.

- Eight running programs; bounded parsed-reply cache and 32 saved state records.
- State identity includes session/turn scope and exact source. Animation does not
  write state to disk. Reset replaces the generation; stale callbacks are rejected.
- Mounted-card visibility covers text-only animations as well as canvases. Hidden,
  blurred, navigated and unmounted controls release transient input; legacy drags
  roll back. Phased input releases without undoing unrelated gameplay.
- Canvas fitting, gradients, SVG strokes, geometry caching and physical-pixel
  subdivision are identical across both apps. Images/WebViews are not involved.

The embedding host must cancel its route on navigation/blur and must never derive
execution permission from assistant content, attachment labels, or a streaming fence.
Desktop additionally requires unambiguous matching durable history entries and
bounds retained native reply projections to eight rows / 256 KiB per transcript.
No hidden native source is parsed as a Markdown document.

See [`pi_visual`](../pi_visual/README.md) for APIs, safeguards and limitations.
Run `cargo test -p pi_visual_ui` for shared native tests; also test with Android
features and the actual frontend adapters. These are in-process cooperative
safeguards, not OS isolation or hard native tessellation-time guarantees.
