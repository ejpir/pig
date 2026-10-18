package dev.pi.gpui;

import android.graphics.Color;
import android.view.Gravity;
import android.view.KeyEvent;
import android.view.MotionEvent;
import android.view.View;
import android.view.ViewConfiguration;
import android.view.WindowManager;
import android.webkit.GeolocationPermissions;
import android.webkit.PermissionRequest;
import android.webkit.ValueCallback;
import android.webkit.WebChromeClient;
import android.webkit.WebResourceRequest;
import android.webkit.WebResourceResponse;
import android.webkit.WebSettings;
import android.webkit.WebView;
import android.webkit.WebViewClient;
import android.widget.FrameLayout;
import android.widget.PopupWindow;
import org.json.JSONArray;
import org.json.JSONObject;
import java.io.ByteArrayInputStream;
import java.util.LinkedHashMap;
import java.util.UUID;

/** App-attached child windows: NativeActivity owns the main surface/input queue.
 * No native bridge is installed in any page. Only Android-originated touch
 * gestures may be forwarded to the native thread, never JavaScript messages.
 */
final class InlinePageHost {
    private static final int MAX_DOCUMENTS = 8;
    private static final int MAX_VIEWS = 3;
    // Headers cannot be relaxed by the document. An opaque sandbox origin and
    // blocked child realms prevent recovering the WebRTC constructors removed
    // by the first trusted script (CSP connect-src alone does not cover RTC).
    private static final String CSP = "default-src 'none'; script-src 'unsafe-inline' 'unsafe-eval'; "
            + "style-src 'unsafe-inline'; img-src data: blob:; font-src data:; media-src data: blob:; "
            + "connect-src 'none'; frame-src 'none'; child-src 'none'; worker-src 'none'; "
            + "object-src 'none'; base-uri 'none'; form-action 'none'; sandbox allow-scripts";
    private static final String BOOTSTRAP = "<!doctype html><script>"
            + "for(const n of ['RTCPeerConnection','webkitRTCPeerConnection','RTCIceGatherer','RTCIceTransport'])"
            + "Object.defineProperty(window,n,{value:undefined,writable:false,configurable:false});"
            + "</script>";
    private final GpuiActivity activity;
    private final Object lock = new Object();
    private final LinkedHashMap<String, Document> documents = new LinkedHashMap<>(8, .75f, true);
    // Views and all Android objects are owned exclusively by the UI thread.
    private final LinkedHashMap<String, Page> pages = new LinkedHashMap<>(4, .75f, true);
    private String pending = "{}";
    private boolean queued;
    private boolean resumed;
    private volatile boolean destroyed;
    private volatile boolean failed;
    private Page active;

    private static final class Document {
        final String revision, html;
        Document(String revision, String html) { this.revision = revision; this.html = html; }
    }

    InlinePageHost(GpuiActivity activity) { this.activity = activity; }

    boolean isAvailable() { return !destroyed && !failed; }

    boolean hasDocument(String id, String revision) {
        synchronized (lock) {
            Document document = documents.get(id);
            return isAvailable() && document != null && document.revision.equals(revision);
        }
    }
    boolean prepare(String id, String revision, String html) {
        if (!isAvailable() || id == null || id.length() > 128 || revision == null
                || revision.length() > 128 || html == null || html.length() > 2 * 1024 * 1024) return false;
        synchronized (lock) {
            documents.put(id, new Document(revision, html));
            while (documents.size() > MAX_DOCUMENTS) documents.remove(documents.keySet().iterator().next());
        }
        return true;
    }
    boolean sync(String json) {
        if (!isAvailable() || json == null || json.length() > 4096) return false;
        synchronized (lock) {
            pending = json;
            if (queued) return true;
            queued = true;
        }
        activity.runOnUiThread(() -> {
            String snapshot;
            synchronized (lock) { snapshot = pending; queued = false; }
            if (!isAvailable()) return;
            try { apply(snapshot); }
            catch (RuntimeException | LinkageError | org.json.JSONException error) {
                fail("Could not create or position the inline WebView", error);
            }
        });
        return true;
    }
    private static int[] rectangle(JSONArray values) throws org.json.JSONException {
        if (values.length() != 4) throw new IllegalArgumentException("Invalid page rectangle");
        double x = values.getDouble(0), y = values.getDouble(1), w = values.getDouble(2), h = values.getDouble(3);
        if (!Double.isFinite(x) || !Double.isFinite(y) || !Double.isFinite(w) || !Double.isFinite(h)
                || Math.abs(x) > 100000 || Math.abs(y) > 100000 || w < 1 || h < 1 || w > 16384 || h > 16384)
            throw new IllegalArgumentException("Invalid page geometry");
        return new int[] {(int)Math.round(x), (int)Math.round(y), (int)Math.round(w), (int)Math.round(h)};
    }
    private void apply(String snapshot) throws org.json.JSONException {
        JSONObject slot = new JSONObject(snapshot);
        if (!resumed || !slot.has("id")) { hide(); return; }
        String id = slot.getString("id"), revision = slot.getString("revision");
        Document document;
        synchronized (lock) { document = documents.get(id); }
        if (document == null || !document.revision.equals(revision)) { hide(); return; }
        int[] bounds = rectangle(slot.getJSONArray("bounds")), clip = rectangle(slot.getJSONArray("clip"));
        if (clip[0] < bounds[0] - 1 || clip[1] < bounds[1] - 1
                || clip[0] + clip[2] > bounds[0] + bounds[2] + 1
                || clip[1] + clip[3] > bounds[1] + bounds[3] + 1) { hide(); return; }
        Page page = pages.get(id);
        if (page != null && !page.revision.equals(revision)) {
            if (active == page) active = null;
            page.destroy(); pages.remove(id); page = null;
        }
        if (page == null) {
            while (pages.size() >= MAX_VIEWS) {
                String oldest = pages.keySet().iterator().next();
                Page removed = pages.remove(oldest);
                if (active == removed) active = null;
                removed.destroy();
            }
            page = new Page(id, document);
            pages.put(id, page);
        }
        if (active != null && active != page) active.hide();
        active = page;
        page.show(bounds, clip, slot.optBoolean("dark", false),
                slot.optBoolean("expanded", false));
    }
    void releaseFocus() {
        activity.runOnUiThread(() -> { if (active != null) active.releaseFocus(); });
    }
    void terminateForTest() {
        activity.runOnUiThread(() -> {
            // VIEW delivery briefly pauses/hides the popup. Terminate its
            // retained renderer even during that lifecycle transition.
            Page candidate = active;
            if (candidate == null) for (Page page : pages.values()) candidate = page;
            if (candidate != null && candidate.web.getWebViewRenderProcess() != null)
                candidate.web.getWebViewRenderProcess().terminate();
        });
    }
    // Harness-initiated readback, never a JavascriptInterface. Android
    // accessibility can expose only a replacement WebView's title until its
    // first DOM change. Read-only DOM/geometry assertions must not press AC to
    // manufacture the zero they are checking after an explicit revision reset.
    void inspectForTest(String request) {
        if (request == null || !request.matches("[0-9]{1,32}")) return;
        activity.runOnUiThread(() -> {
            Page page = active;
            if (page == null || page.retired) return;
            String script = "JSON.stringify({ready:document.readyState,theme:document.documentElement.getAttribute('data-pi-theme'),"
                    + "width:innerWidth,result:document.getElementById('result')?.textContent?.slice(0,128),"
                    + "input:document.querySelector('input')?.value?.slice(0,128),active:document.activeElement?.tagName,"
                    + "controls:Array.from(document.querySelectorAll('button,input')).slice(0,64).map(e=>{"
                    + "const r=e.getBoundingClientRect();return {label:(e.getAttribute('aria-label')||e.textContent||'').slice(0,128),"
                    + "rect:[r.x,r.y,r.width,r.height],type:e.tagName}})})";
            page.web.evaluateJavascript(script, value -> {
                if (page != active || page.retired || value == null || value.equals("null")) return;
                try {
                    JSONObject state = new JSONObject(new JSONArray("[" + value + "]").getString(0));
                    int[] origin = new int[2]; page.web.getLocationOnScreen(origin);
                    state.put("request", request); state.put("id", page.id); state.put("revision", page.revision);
                    state.put("origin", new JSONArray(origin));
                    state.put("scale", page.web.getWidth() / state.getDouble("width"));
                    java.io.File file = new java.io.File(activity.getFilesDir(), "ui-test-inline.json");
                    java.io.File partial = new java.io.File(activity.getFilesDir(), "ui-test-inline.json.part");
                    try (java.io.FileOutputStream out = new java.io.FileOutputStream(partial)) {
                        out.write(state.toString().getBytes(java.nio.charset.StandardCharsets.UTF_8));
                    }
                    partial.renameTo(file);
                } catch (java.io.IOException | org.json.JSONException error) {
                    android.util.Log.w("PiInlinePage", "Could not inspect fixture document", error);
                }
            });
        });
    }
    private void hide() {
        if (active != null) { active.hide(); active = null; }
    }
    void resume() { resumed = true; }
    void pause() { resumed = false; hide(); }
    void clear() {
        synchronized (lock) { documents.clear(); pending = "{}"; }
        activity.runOnUiThread(() -> {
            boolean changed = failed;
            for (Page page : pages.values()) page.destroy();
            pages.clear(); active = null;
            failed = false; // A deliberate store replacement permits a new host.
            if (changed && !destroyed) activity.inlineFailed();
        });
    }
    void destroy() { destroyed = true; clear(); }

    private void fail(String reason, Throwable error) {
        if (failed || destroyed) return;
        failed = true; // Stop retries, including a document that crashes on load.
        android.util.Log.w("PiInlinePage", reason, error);
        for (Page page : pages.values()) page.destroy();
        pages.clear(); active = null;
        activity.inlineFailed();
    }

    private final class Page {
        final String id, revision;
        final WebView web;
        final PageClip root;
        final PopupWindow popup;
        boolean internallyDismissed;
        boolean paused = true;
        boolean loaded;
        boolean retired;
        boolean dark;
        boolean expanded;
        int[] lastBounds, lastClip;

        @SuppressWarnings("deprecation")
        Page(String id, Document document) {
            this.id = id; revision = document.revision;
            web = new WebView(activity);
            web.setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_YES);
            web.setBackgroundColor(Color.WHITE);
            WebSettings settings = web.getSettings();
            settings.setJavaScriptEnabled(true);
            settings.setDomStorageEnabled(false);
            settings.setAllowFileAccess(false);
            settings.setAllowContentAccess(false);
            settings.setAllowFileAccessFromFileURLs(false);
            settings.setAllowUniversalAccessFromFileURLs(false);
            settings.setBlockNetworkLoads(true);
            settings.setMixedContentMode(WebSettings.MIXED_CONTENT_NEVER_ALLOW);
            settings.setSupportMultipleWindows(false);
            settings.setJavaScriptCanOpenWindowsAutomatically(false);
            settings.setGeolocationEnabled(false);
            settings.setMediaPlaybackRequiresUserGesture(true);
            settings.setSaveFormData(false);
            settings.setCacheMode(WebSettings.LOAD_NO_CACHE);
            web.removeJavascriptInterface("searchBoxJavaBridge_");
            web.removeJavascriptInterface("accessibility");
            web.removeJavascriptInterface("accessibilityTraversal");
            android.webkit.CookieManager.getInstance().setAcceptThirdPartyCookies(web, false);
            final String documentUrl = "https://" + UUID.randomUUID() + ".invalid/document";
            final byte[] bytes = (BOOTSTRAP + document.html).getBytes(java.nio.charset.StandardCharsets.UTF_8);
            final java.util.Map<String, String> headers = new java.util.HashMap<>();
            headers.put("Content-Security-Policy", CSP);
            headers.put("Referrer-Policy", "no-referrer");
            headers.put("Cache-Control", "no-store");
            headers.put("X-DNS-Prefetch-Control", "off");
            headers.put("Permissions-Policy", "camera=(), microphone=(), geolocation=(), display-capture=(), "
                    + "usb=(), serial=(), bluetooth=(), payment=(), clipboard-read=(), clipboard-write=(), "
                    + "web-share=(), fullscreen=(), screen-wake-lock=(), xr-spatial-tracking=(), "
                    + "accelerometer=(), gyroscope=(), magnetometer=(), midi=()");
            web.setWebViewClient(new WebViewClient() {
                @Override public boolean shouldOverrideUrlLoading(WebView view, WebResourceRequest request) {
                    return true; // Automatic inline execution never opens an external app.
                }
                @Override public boolean shouldOverrideUrlLoading(WebView view, String url) { return true; }
                @Override public WebResourceResponse shouldInterceptRequest(WebView view, WebResourceRequest request) {
                    if (request.isForMainFrame() && documentUrl.equals(request.getUrl().toString())) {
                        return new WebResourceResponse("text/html", "UTF-8", 200, "OK", headers,
                                new ByteArrayInputStream(bytes));
                    }
                    String scheme = request.getUrl().getScheme();
                    if ("data".equals(scheme) || "blob".equals(scheme)) return null;
                    return new WebResourceResponse("text/plain", "UTF-8", 403, "Blocked",
                            java.util.Collections.emptyMap(), new ByteArrayInputStream(new byte[0]));
                }
                @Override public boolean onRenderProcessGone(WebView view, android.webkit.RenderProcessGoneDetail detail) {
                    if (!retired) fail("Inline renderer stopped; preview disabled until computer reload", null);
                    return true; // A browser renderer exit must not kill the native chat.
                }
                @Override public void onReceivedError(WebView view, WebResourceRequest request, android.webkit.WebResourceError error) {
                    if (request.isForMainFrame() && !retired) fail("Inline document could not load", null);
                }
                @Override public void onPageFinished(WebView view, String url) {
                    if (retired) return;
                    loaded = true; applyTheme();
                    view.sendAccessibilityEvent(android.view.accessibility.AccessibilityEvent.TYPE_WINDOW_CONTENT_CHANGED);
                }
            });
            web.setWebChromeClient(new WebChromeClient() {
                @Override public boolean onJsAlert(WebView view, String url, String message, android.webkit.JsResult result) { result.cancel(); return true; }
                @Override public boolean onJsConfirm(WebView view, String url, String message, android.webkit.JsResult result) { result.cancel(); return true; }
                @Override public boolean onJsPrompt(WebView view, String url, String message, String value, android.webkit.JsPromptResult result) { result.cancel(); return true; }
                @Override public boolean onJsBeforeUnload(WebView view, String url, String message, android.webkit.JsResult result) { result.confirm(); return true; }
                @Override public void onPermissionRequest(PermissionRequest request) { request.deny(); }
                @Override public void onGeolocationPermissionsShowPrompt(String origin, GeolocationPermissions.Callback callback) {
                    callback.invoke(origin, false, false);
                }
                @Override public boolean onShowFileChooser(WebView view, ValueCallback<android.net.Uri[]> callback, FileChooserParams params) {
                    callback.onReceiveValue(null); return true;
                }
                @Override public boolean onCreateWindow(WebView view, boolean dialog, boolean gesture, android.os.Message message) { return false; }
            });
            web.setDownloadListener((url, agent, disposition, mime, length) -> {});
            root = new PageClip(this);
            root.setClipChildren(true); root.setClipToPadding(true);
            root.addView(web);
            root.setOnApplyWindowInsetsListener((view, insets) -> { activity.inlineInsets(insets); return insets; });
            popup = new PopupWindow(root, 1, 1, false);
            popup.setWindowLayoutType(WindowManager.LayoutParams.TYPE_APPLICATION_PANEL);
            popup.setIsLaidOutInScreen(false);
            popup.setAttachedInDecor(false);
            popup.setClippingEnabled(false);
            popup.setAnimationStyle(0);
            popup.setElevation(0);
            popup.setBackgroundDrawable(null);
            popup.setTouchModal(false);
            popup.setOutsideTouchable(false);
            popup.setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_ADJUST_NOTHING);
            popup.setOnDismissListener(() -> {
                if (!internallyDismissed && resumed && !destroyed && !retired) activity.inlineBack();
            });
            // Main-frame interception supplies trusted CSP response headers,
            // including sandbox (which is ignored in a meta element). No actual
            // DNS/HTTP request, app origin or neighboring file is used.
            web.loadUrl(documentUrl);
        }
        void show(int[] bounds, int[] clip, boolean nextDark, boolean nextExpanded) {
            boolean changed = lastBounds == null || !java.util.Arrays.equals(bounds, lastBounds)
                    || !java.util.Arrays.equals(clip, lastClip);
            expanded = nextExpanded;
            if (nextDark != dark || !loaded) { dark = nextDark; applyTheme(); }
            if (changed) {
                FrameLayout.LayoutParams layout = new FrameLayout.LayoutParams(bounds[2], bounds[3]);
                layout.leftMargin = bounds[0] - clip[0]; layout.topMargin = bounds[1] - clip[1];
                web.setLayoutParams(layout);
                lastBounds = bounds; lastClip = clip;
            }
            if (paused) { web.onResume(); paused = false; }
            if (!popup.isShowing()) {
                popup.setWidth(clip[2]); popup.setHeight(clip[3]);
                popup.showAtLocation(activity.getWindow().getDecorView(), Gravity.TOP | Gravity.LEFT, clip[0], clip[1]);
            } else if (changed) popup.update(clip[0], clip[1], clip[2], clip[3]);
        }
        void applyTheme() {
            web.setBackgroundColor(dark ? 0xff202731 : Color.WHITE);
            // A presentation signal, not a bridge. Documents can opt into this
            // attribute; arbitrary page state is never migrated between revisions.
            if (loaded && !retired) web.evaluateJavascript("document.documentElement.setAttribute('data-pi-theme','" + (dark ? "dark" : "light") + "');document.documentElement.style.colorScheme='" + (dark ? "dark" : "light") + "'", null);
        }
        void focus() {
            if (!popup.isFocusable()) { popup.setFocusable(true); popup.update(); }
            web.requestFocus();
            web.sendAccessibilityEvent(android.view.accessibility.AccessibilityEvent.TYPE_WINDOW_CONTENT_CHANGED);
        }
        void releaseFocus() {
            if (popup.isFocusable()) { popup.setFocusable(false); popup.update(); }
        }
        void hide() {
            releaseFocus();
            internallyDismissed = true;
            popup.dismiss();
            internallyDismissed = false;
            if (!paused) { web.onPause(); paused = true; }
        }
        void destroy() {
            if (retired) return;
            retired = true;
            hide(); root.removeView(web); web.destroy();
        }
    }

    /** Compact previews never become nested vertical scroll traps: their
     * vertical drags belong to the chat even when the document is taller than
     * its viewport. Expanded pages keep their own scrolling. ViewGroup
     * interception cancels the WebView press before forwarding the claimed
     * gesture, so scrolling cannot also activate a page control.
     */
    private final class PageClip extends FrameLayout {
        final Page page;
        final int slop;
        float downX, downY;
        boolean scrolling;
        PageClip(Page page) { super(activity); this.page = page; slop = ViewConfiguration.get(activity).getScaledTouchSlop(); }
        private void forward(int action, float x, float y) {
            int[] origin = new int[2]; activity.getWindow().getDecorView().getLocationOnScreen(origin);
            activity.inlineTouch(action, x - origin[0], y - origin[1]);
        }
        @Override public boolean onInterceptTouchEvent(MotionEvent event) {
            if (event.getActionMasked() == MotionEvent.ACTION_DOWN) {
                scrolling = false; downX = event.getRawX(); downY = event.getRawY(); page.focus();
            } else if (event.getActionMasked() == MotionEvent.ACTION_MOVE) {
                float dy = event.getRawY() - downY, dx = event.getRawX() - downX;
                if (event.getPointerCount() == 1 && Math.abs(dy) > slop
                        && Math.abs(dy) > Math.abs(dx) && !page.expanded) {
                    scrolling = true;
                    forward(MotionEvent.ACTION_DOWN, downX, downY);
                    return true;
                }
            }
            return false;
        }
        @Override public boolean onTouchEvent(MotionEvent event) {
            if (!scrolling) return super.onTouchEvent(event);
            forward(event.getActionMasked(), event.getRawX(), event.getRawY());
            if (event.getActionMasked() == MotionEvent.ACTION_UP || event.getActionMasked() == MotionEvent.ACTION_CANCEL) scrolling = false;
            return true;
        }
        @Override public boolean dispatchKeyEvent(KeyEvent event) {
            if (event.getKeyCode() == KeyEvent.KEYCODE_BACK) {
                if (event.getAction() == KeyEvent.ACTION_UP && !event.isCanceled()) activity.inlineBack();
                return true;
            }
            return super.dispatchKeyEvent(event);
        }
    }
}
