# Inline pages · Pi for Android

**[Open the interactive, self-contained study](index.html)** · [Phone-only view](index.html?single=1) · [Overview PNG](overview.png)

A proposal for HTML pages that work **inside the conversation**, rather than cards that must be opened in another screen. The default screen contains a real, usable calculator in a sandboxed iframe. The surrounding session, revisions and connection states are sample content; this is not implemented Android UI.

## Try it

- Calculate directly in the reply. **AC**, digits, decimal, sign, percent, arithmetic and keyboard input work; **Enter** evaluates when the calculator background is focused. Enter/Space on a focused key activate that key normally.
- **Expand** gives the same page more space. Back returns to the thread without recreating the iframe or losing the calculation.
- **Source** opens selectable, copyable HTML in a sheet, without changing the page.
- Choose **A revision is ready**. Your current calculation remains active. **Update → Keep v1** preserves it; **Load v2** explicitly starts a fresh document with working memory buttons.
- Choose **The computer is away**. Local calculations still work. Drafts stay editable, but Send is disabled. Reconnect only changes this local sample; it does not contact a computer.
- Choose **An edit we can’t follow**. No old preview is silently shown. **View saved v1 instead** explicitly opens a labeled historical snapshot. **Ask Pi to rewrite it** only fills a draft.
- The composer can simulate a follow-up and show the sample revision state. It never submits a real prompt; sample follow-ups are labeled accordingly.
- Switch between Moonstone and Evening without reloading the calculator. System dark mode sets the initial theme.

Escape closes a sheet before leaving expanded mode. Sheets trap focus and make the underlying study inert; closing returns focus to the invoking control. Calculator keys and artifact actions have at least 48px touch targets. In short landscape windows, the expanded page scrolls within its bounded surface.

## Screen study

| Screen | Interaction |
| --- | --- |
| [01 · In the conversation](screens/01-inline.png) | The working calculator is between Pi’s introduction and closing words. Only the page is a web surface. |
| [02 · Revision ready](screens/02-update.png) | v2 waits while v1 remains usable. Replacing the document requires an explicit action and warns that page-local state resets. |
| [03 · Expanded](screens/03-expanded.png) | Same page, more room. No new browser destination or load. |
| [04 · Source](screens/04-source.png) | Selectable source in a native-style bottom sheet. |
| [05 · Computer offline](screens/05-offline.png) | Cached self-contained HTML keeps working; agent submission remains unavailable. |
| [06 · Revision unavailable](screens/06-unavailable.png) | Unknown contents are named honestly; saved v1 is a separately labeled choice. |
| [07 · Evening](screens/07-evening.png) | The surrounding UI and calculator respond to theme changes. |

## Design contract

1. **No opening gate.** A captured, self-contained page can be used directly where its `Flow::Page` occurs in the reply. Expand is an optional affordance, not the default journey.
2. **Native ownership outside the frame.** Pi owns navigation, artifact identity/version, permissions, source and composer. The page owns only its content and interaction.
3. **Stable running documents.** Scrolling, streaming chat changes, inspecting source, themes and expanding do not recreate an unchanged document. An applied HTML revision does. The study retains its single iframe; it does not demonstrate native offscreen-view eviction or persistence after process death.
4. **Updates do not silently erase input.** This study pins the running version until the user chooses to load a new one. It makes no promise to migrate arbitrary DOM state between revisions.
5. **Unknown is not unchanged.** A reconstruction failure removes the current preview. A historical page is only available via an explicit, clearly labeled saved-version action.
6. **Local capability by default.** Scripts are allowed; network resources, filesystem, credentials and native app APIs are not. A page cannot send prompts, call agent tools or inherit the host account’s permissions. Network-enabled pages would need a separate permission design; no network permission grant is simulated here.
7. **Offline is scoped.** A cached self-contained calculator works offline. That does not imply the agent, remote file edits or other network-dependent pages are available.

## Implementation boundary

The app reconstructs `.html`/`.htm` from successful writes and exact edits in `pi_markdown::Pages`, and positions pages using `Flow::Page`. It now has a native inline implementation: the conversation remains GPUI while an app-attached, clipped `PopupWindow`/`WebView` hosts the latest artifact. Source, revision confirmation and permissions remain GPUI. Older cards and Open separately retain the existing viewer. See [native architecture, limits and emulator evidence](../../../crates/pi_android/scripts/INLINE_PAGES_VERIFICATION.md). This browser study is separate from that implementation.

This prototype uses an actual HTML `iframe` with `sandbox="allow-scripts"` (no same-origin permission), `referrerpolicy="no-referrer"` and a document CSP that blocks external resources and connections. Theme messages go inward; bounded display text comes outward. Messages are accepted only from the expected window. No app tools or network APIs are exposed by the parent. This browser boundary illustrates the intended policy; it is **not** an implementation or security proof of the native WebView sandbox.

GPUI is not a DOM; ordinary child views cannot compose reliably over its NativeActivity surface. The native host now receives actual painted bounds, clips to the chat viewport, hides beneath sheets/notices, coordinates browser/GPUI touch and keyboard focus, and retains pinned documents across geometry and theme changes. It bounds retention to three WebViews/eight documents, with a 2 MiB HTML limit and documented cache/store/process reset behaviour.

Automatic inline execution uses a trusted response CSP, opaque unrelated origins, RTC/child-realm restrictions, DNS-prefetch suppression, and native network/navigation/resource/permission controls. The new envelope is scoped to the inline host, not legacy browser paths. These WebView controls are not a formal OS network/resource sandbox.

Native API 35 x86_64 emulator verification passed **80 inline checks**, including actual renderer termination and explicit recovery. This is separate from this folder’s browser checks. Reports/screenshots are under `artifacts/inline-pages-emulator/final/`; remaining platform/accessibility/composer coverage is documented in the linked notes. No physical-device deployment was performed.

## Reproduce

Editable sources are `index.template`, `inline-pages.css` and `inline-pages.js`. The generated `index.html` embeds styles, scripts and the repository fonts. The calculator HTML is generated locally and embedded with `srcdoc`; no neighboring files or HTTP server are needed for the experience.

```sh
node design/android/inline-pages/build.cjs
node design/android/inline-pages/build.cjs --check
node design/android/inline-pages/render.cjs          # checks + screenshots
node design/android/inline-pages/render.cjs --check # checks only
```

Browser checks require an existing Puppeteer/Puppeteer Core and Chromium installation (`CHROME` may specify the executable). No dependencies are installed by these scripts. Checks cover arithmetic/memory/keyboard, retained state, explicit revisions, dialogs/focus, source, themes, offline/drafts, saved-version labeling, input escaping, responsive layouts and an Android-style HTML-only preview with all external requests blocked. These are browser prototype checks, not native Android/emulator validation.
