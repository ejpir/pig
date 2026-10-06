package dev.pi.gpui;

import android.app.Activity;
import android.content.Context;
import android.content.Intent;
import android.graphics.Bitmap;
import android.graphics.Color;
import android.graphics.Insets;
import android.graphics.Typeface;
import android.graphics.drawable.Drawable;
import android.graphics.drawable.GradientDrawable;
import android.net.Uri;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.text.TextUtils;
import android.view.Gravity;
import android.view.PixelCopy;
import android.view.View;
import android.view.ViewGroup;
import android.view.WindowInsets;
import android.view.WindowInsetsController;
import android.webkit.WebResourceRequest;
import android.webkit.WebSettings;
import android.webkit.WebView;
import android.webkit.WebViewClient;
import android.widget.FrameLayout;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;
import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.util.UUID;

/** Sandboxed preview and selectable source for an HTML page created by Pi. */
public final class PageActivity extends Activity {
    private static final String TAG = "PageActivity";
    public static final String TITLE = "title";
    public static final String PATH = "path";
    public static final String DARK = "dark";
    /** Opens on the source rather than the page. */
    public static final String SOURCE = "source";
    /** Where to save a picture of the page, for its card in the thread. */
    public static final String POSTER = "poster";

    public static File save(Context context, String html) throws IOException {
        File directory = new File(context.getCacheDir(), "pi-pages");
        if (!directory.isDirectory() && !directory.mkdirs()) {
            throw new IOException("Could not create page cache");
        }
        File file = new File(directory, UUID.randomUUID() + ".html");
        try (FileOutputStream output = new FileOutputStream(file)) {
            output.write(html.getBytes(StandardCharsets.UTF_8));
        }
        return file;
    }

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        android.util.Log.i(TAG, "Opening the sandboxed HTML page viewer");
        boolean dark = getIntent().getBooleanExtra(DARK, false);
        int canvas = dark ? Color.rgb(22, 29, 39) : Color.rgb(250, 249, 247);
        int text = dark ? Color.rgb(235, 231, 228) : Color.rgb(37, 47, 61);
        int muted = dark ? Color.rgb(156, 162, 170) : Color.rgb(102, 95, 89);
        int secondary = dark ? Color.rgb(213, 216, 219) : Color.rgb(62, 71, 83);
        int line = dark ? Color.rgb(54, 61, 70) : Color.rgb(221, 215, 208);
        int chip = dark ? Color.rgb(32, 39, 49) : Color.WHITE;
        int segments = dark ? Color.rgb(26, 33, 43) : Color.rgb(240, 237, 232);
        getWindow().setStatusBarColor(canvas);
        getWindow().setNavigationBarColor(canvas);

        String html = readAndDelete(getIntent().getStringExtra(PATH));
        LinearLayout root = new LinearLayout(this);
        root.setOrientation(LinearLayout.VERTICAL);
        root.setBackgroundColor(canvas);
        root.setOnApplyWindowInsetsListener(
                (view, windowInsets) -> {
                    Insets insets =
                            windowInsets.getInsets(
                                    WindowInsets.Type.systemBars()
                                            | WindowInsets.Type.displayCutout());
                    view.setPadding(insets.left, insets.top, insets.right, insets.bottom);
                    return windowInsets;
                });

        FrameLayout content = new FrameLayout(this);
        WebView preview = new WebView(this);
        preview.setBackgroundColor(Color.WHITE);
        // Pages are interactive, but they get no Android bridge, local files,
        // content providers, persistent storage, or network. JavaScript can
        // manipulate only the self-contained document supplied by Pi.
        preview.getSettings().setJavaScriptEnabled(true);
        preview.getSettings().setAllowContentAccess(false);
        preview.getSettings().setAllowFileAccess(false);
        preview.getSettings().setAllowFileAccessFromFileURLs(false);
        preview.getSettings().setAllowUniversalAccessFromFileURLs(false);
        preview.getSettings().setBlockNetworkLoads(true);
        preview.getSettings().setDomStorageEnabled(false);
        preview.getSettings().setDatabaseEnabled(false);
        preview.getSettings().setGeolocationEnabled(false);
        preview.getSettings().setMediaPlaybackRequiresUserGesture(true);
        preview.getSettings().setMixedContentMode(WebSettings.MIXED_CONTENT_NEVER_ALLOW);
        preview.removeJavascriptInterface("searchBoxJavaBridge_");
        preview.removeJavascriptInterface("accessibility");
        preview.removeJavascriptInterface("accessibilityTraversal");
        String poster = getIntent().getStringExtra(POSTER);
        preview.setWebViewClient(new WebViewClient() {
            @Override
            public void onPageFinished(WebView view, String url) {
                // Give scripts a moment to draw, then keep how the page looks.
                if (poster != null && !poster.isEmpty() && !new File(poster).exists()) {
                    view.postDelayed(() -> savePoster(view, poster), 1500);
                }
            }

            @Override
            public boolean shouldOverrideUrlLoading(WebView view, WebResourceRequest request) {
                openExternal(request.getUrl());
                return true;
            }

            @Override
            @SuppressWarnings("deprecation")
            public boolean shouldOverrideUrlLoading(WebView view, String url) {
                openExternal(Uri.parse(url));
                return true;
            }
        });
        preview.loadDataWithBaseURL(null, html, "text/html", "UTF-8", null);
        content.addView(preview, new FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));

        ScrollView sourceScroll = new ScrollView(this);
        sourceScroll.setVisibility(View.GONE);
        TextView source = label(html, 13, text);
        source.setTypeface(Typeface.MONOSPACE);
        source.setTextIsSelectable(true);
        source.setPadding(dp(16), dp(12), dp(16), dp(24));
        sourceScroll.addView(source, new ScrollView.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT));
        content.addView(sourceScroll, new FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
        root.addView(content, new LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, 0, 1));

        // The tools sit at the bottom, under the thumb: close, Page or
        // Source, and reload.
        View rule = new View(this);
        rule.setBackgroundColor(line);
        root.addView(rule, new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, dp(1)));
        LinearLayout bar = new LinearLayout(this);
        bar.setGravity(Gravity.CENTER_VERTICAL);
        bar.setPadding(dp(8), 0, dp(8), 0);
        TextView close = label("✕", 18, secondary);
        close.setGravity(Gravity.CENTER);
        close.setContentDescription("Close");
        close.setOnClickListener(view -> finish());
        bar.addView(close, new LinearLayout.LayoutParams(dp(48), dp(48)));
        LinearLayout tabs = new LinearLayout(this);
        tabs.setPadding(dp(4), dp(4), dp(4), dp(4));
        tabs.setBackground(rounded(segments, dp(20)));
        TextView previewTab = tab("Page", text, rounded(chip, dp(16)));
        TextView sourceTab = tab("Source", muted, null);
        tabs.addView(previewTab, new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.MATCH_PARENT, 1));
        tabs.addView(sourceTab, new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.MATCH_PARENT, 1));
        LinearLayout.LayoutParams tabsLayout = new LinearLayout.LayoutParams(0, dp(40), 1);
        tabsLayout.setMargins(dp(8), 0, dp(8), 0);
        bar.addView(tabs, tabsLayout);
        TextView reload = label("↻", 22, secondary);
        reload.setGravity(Gravity.CENTER);
        reload.setContentDescription("Reload");
        reload.setOnClickListener(view -> preview.loadDataWithBaseURL(null, html, "text/html", "UTF-8", null));
        bar.addView(reload, new LinearLayout.LayoutParams(dp(48), dp(48)));
        root.addView(bar, new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, dp(64)));

        previewTab.setOnClickListener(view -> {
            preview.setVisibility(View.VISIBLE);
            sourceScroll.setVisibility(View.GONE);
            previewTab.setTextColor(text);
            previewTab.setBackground(rounded(chip, dp(16)));
            sourceTab.setTextColor(muted);
            sourceTab.setBackground(null);
        });
        sourceTab.setOnClickListener(view -> {
            preview.setVisibility(View.GONE);
            sourceScroll.setVisibility(View.VISIBLE);
            sourceTab.setTextColor(text);
            sourceTab.setBackground(rounded(chip, dp(16)));
            previewTab.setTextColor(muted);
            previewTab.setBackground(null);
        });
        setContentView(root);
        WindowInsetsController bars = getWindow().getInsetsController();
        if (bars != null) {
            int light =
                    WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS
                            | WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS;
            bars.setSystemBarsAppearance(dark ? 0 : light, light);
        }
        root.requestApplyInsets();
        if (getIntent().getBooleanExtra(SOURCE, false)) {
            sourceTab.performClick();
        }
    }

    /** The page as it shows, at half size; its card shows the middle of it. */
    private void savePoster(WebView view, String path) {
        if (isFinishing() || view.getWidth() == 0 || view.getVisibility() != View.VISIBLE) return;
        int[] at = new int[2];
        view.getLocationInWindow(at);
        int width = view.getWidth();
        int height = view.getHeight();
        Bitmap bitmap = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888);
        android.graphics.Rect area = new android.graphics.Rect(at[0], at[1], at[0] + width, at[1] + height);
        try {
            PixelCopy.request(getWindow(), area, bitmap, result -> {
                if (result != PixelCopy.SUCCESS) return;
                new Thread(() -> {
                    File file = new File(path);
                    File directory = file.getParentFile();
                    if (directory != null) directory.mkdirs();
                    File partial = new File(path + ".part");
                    Bitmap small = Bitmap.createScaledBitmap(bitmap, width / 2, height / 2, true);
                    try (FileOutputStream output = new FileOutputStream(partial)) {
                        small.compress(Bitmap.CompressFormat.PNG, 100, output);
                    } catch (IOException error) {
                        return;
                    }
                    partial.renameTo(file);
                }).start();
            }, new Handler(Looper.getMainLooper()));
        } catch (IllegalArgumentException ignored) {
        }
    }

    private String readAndDelete(String path) {
        if (path == null) return "";
        File file = new File(path);
        try (FileInputStream input = new FileInputStream(file);
                ByteArrayOutputStream output = new ByteArrayOutputStream()) {
            byte[] buffer = new byte[8192];
            int count;
            while ((count = input.read(buffer)) != -1) {
                output.write(buffer, 0, count);
            }
            return new String(output.toByteArray(), StandardCharsets.UTF_8);
        } catch (IOException error) {
            return "<p>This page could not be loaded.</p>";
        } finally {
            file.delete();
        }
    }

    private void openExternal(Uri uri) {
        String scheme = uri == null ? null : uri.getScheme();
        if (!"http".equals(scheme) && !"https".equals(scheme) && !"mailto".equals(scheme)) return;
        try {
            startActivity(new Intent(Intent.ACTION_VIEW, uri));
        } catch (RuntimeException ignored) {
        }
    }

    private TextView tab(String text, int color, Drawable background) {
        TextView view = label(text, 14, color);
        view.setGravity(Gravity.CENTER);
        view.setTypeface(Typeface.DEFAULT, Typeface.BOLD);
        view.setBackground(background);
        return view;
    }

    private static Drawable rounded(int color, int radius) {
        GradientDrawable shape = new GradientDrawable();
        shape.setColor(color);
        shape.setCornerRadius(radius);
        return shape;
    }

    private TextView label(String text, int sp, int color) {
        TextView view = new TextView(this);
        view.setText(text == null ? "Page" : text);
        view.setTextSize(sp);
        view.setTextColor(color);
        return view;
    }

    private int dp(int value) {
        return Math.round(value * getResources().getDisplayMetrics().density);
    }

    @Override
    protected void onDestroy() {
        View root = findViewById(android.R.id.content);
        destroyWebViews(root);
        super.onDestroy();
    }

    private void destroyWebViews(View view) {
        if (view instanceof WebView) {
            ((WebView) view).destroy();
        } else if (view instanceof ViewGroup) {
            ViewGroup group = (ViewGroup) view;
            for (int i = 0; i < group.getChildCount(); i++) destroyWebViews(group.getChildAt(i));
        }
    }
}
