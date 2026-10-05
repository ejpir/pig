package dev.pi.gpui;

import android.app.NativeActivity;
import android.content.ActivityNotFoundException;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.ComponentName;
import android.content.Intent;
import android.content.pm.ActivityInfo;
import android.content.pm.PackageManager;
import android.content.res.Configuration;
import android.graphics.Color;
import android.graphics.Insets;
import android.net.Uri;
import android.os.Bundle;
import android.text.Editable;
import android.text.Selection;
import android.text.SpannableStringBuilder;
import android.text.Spanned;
import android.text.style.UnderlineSpan;
import android.util.Log;
import android.view.Display;
import android.view.HapticFeedbackConstants;
import android.view.View;
import android.view.ViewGroup;
import android.view.WindowInsets;
import android.view.WindowInsetsAnimation;
import android.view.WindowInsetsController;
import android.view.WindowManager;
import android.view.inputmethod.BaseInputConnection;
import android.webkit.MimeTypeMap;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import android.view.inputmethod.InputMethodManager;
import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.atomic.AtomicInteger;

/**
 * A NativeActivity with what Android offers only to Java: the on-screen keyboard,
 * window insets, the clipboard, file pickers, opening links, display modes,
 * notifications and the URLs the app is opened with.
 *
 * <p>The keyboard edits {@link #mirror}, a copy of the text around the app's selection.
 * After each batch of edits the mirror's state goes to native code, which applies the
 * difference to the app; native code pushes the app's state back when the app changes
 * it. Each report carries a sequence number and each push names the report it was based
 * on, so a push that crossed a keyboard edit is dropped instead of undoing it.
 *
 * <p>Native code calls the public methods from its own thread.
 */
public class GpuiActivity extends NativeActivity {
    private static final String TAG = "GpuiActivity";

    private static native void nativeImeState(
            int seq,
            int pushId,
            String text,
            int selectionStart,
            int selectionEnd,
            int composingStart,
            int composingEnd);

    private static native void nativeEditorAction(int action);

    /**
     * The result of {@link #pickFiles} or {@link #pickSaveFile}: paths separated by NUL,
     * or null with an error, or both null when the user cancelled.
     */
    private static native void nativePicked(int request, String paths, String error);

    /** A URL the app was opened with, from a link or a notification. */
    private static native void nativeOpenUrl(String url);

    private static native void nativeInsets(
            int barsLeft,
            int barsTop,
            int barsRight,
            int barsBottom,
            int imeLeft,
            int imeTop,
            int imeRight,
            int imeBottom);

    // Keyboard state, only touched on the UI thread.
    private final Editable mirror = new SpannableStringBuilder();
    private final Object composing = new UnderlineSpan();
    private int seq;
    private int batchDepth;
    private boolean edited;
    private int inputType = EditorInfo.TYPE_CLASS_TEXT;
    private int imeOptions = EditorInfo.IME_ACTION_NONE;
    private InputMethodManager inputMethods;
    private EditorView editor;
    // A queued show must not resurrect an editor after navigation dismissed it.
    private final AtomicInteger keyboardRequest = new AtomicInteger();

    /** File picks in progress, by request code: whether each saves. UI thread only. */
    private final Map<Integer, Boolean> picks = new HashMap<>();

    /** Set once native code is listening. */
    private volatile boolean nativeReady;

    /** System bar icons: -1 follows night mode, 0 light icons, 1 dark icons. UI thread only. */
    private int barIcons = -1;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        // Load the library through this class loader too, so the VM finds the native
        // methods above. NativeActivity loads it again, which returns the same library.
        System.loadLibrary(libraryName());
        super.onCreate(savedInstanceState);
        // GPUI owns IME avoidance using animated insets. Replacing only the
        // visibility bits would restore Android's default adjustPan, moving
        // the native input coordinates separately from our rendered surface.
        getWindow().setSoftInputMode(
                WindowManager.LayoutParams.SOFT_INPUT_STATE_ALWAYS_HIDDEN
                        | WindowManager.LayoutParams.SOFT_INPUT_ADJUST_NOTHING);
        Files.clearPicked(this);
        if (savedInstanceState == null) {
            openIntent(getIntent());
        }
        // Draw under the system bars and keyboard; native code avoids them using the
        // insets reported below. Android 15 does this by itself for apps targeting it.
        drawEdgeToEdge();
        inputMethods = getSystemService(InputMethodManager.class);
        editor = new EditorView();
        addContentView(editor, new ViewGroup.LayoutParams(1, 1));
        View decor = getWindow().getDecorView();
        decor.setOnApplyWindowInsetsListener(
                (view, insets) -> {
                    sendInsets(insets);
                    return view.onApplyWindowInsets(insets);
                });
        // Report the keyboard's position on every frame while it slides in or out.
        decor.setWindowInsetsAnimationCallback(
                new WindowInsetsAnimation.Callback(
                        WindowInsetsAnimation.Callback.DISPATCH_MODE_CONTINUE_ON_SUBTREE) {
                    @Override
                    public WindowInsets onProgress(
                            WindowInsets insets, List<WindowInsetsAnimation> running) {
                        sendInsets(insets);
                        return insets;
                    }
                });
    }

    @SuppressWarnings("deprecation")
    private void drawEdgeToEdge() {
        getWindow().setDecorFitsSystemWindows(false);
        getWindow().setStatusBarColor(Color.TRANSPARENT);
        getWindow().setNavigationBarColor(Color.TRANSPARENT);
        followNightMode(getResources().getConfiguration());
    }

    /** Dark system bar icons on light backgrounds, which GPUI apps draw in light mode. */
    private void followNightMode(Configuration configuration) {
        WindowInsetsController controller = getWindow().getInsetsController();
        if (controller == null) {
            return;
        }
        boolean night =
                barIcons == -1
                        ? (configuration.uiMode & Configuration.UI_MODE_NIGHT_MASK)
                                == Configuration.UI_MODE_NIGHT_YES
                        : barIcons == 0;
        int light =
                WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS
                        | WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS;
        controller.setSystemBarsAppearance(night ? 0 : light, light);
    }

    @Override
    public void onConfigurationChanged(Configuration configuration) {
        super.onConfigurationChanged(configuration);
        followNightMode(configuration);
    }

    /**
     * For an app that draws its own theme: {@code 1} for dark system bar icons (a light
     * app), {@code 0} for light icons, {@code -1} to follow the system's night mode.
     */
    public void setBarIcons(int mode) {
        runOnUiThread(
                () -> {
                    barIcons = mode;
                    followNightMode(getResources().getConfiguration());
                });
    }

    /** The short buzz of a long press, such as one that selects text. */
    public void longPressFeedback() {
        runOnUiThread(
                () ->
                        getWindow()
                                .getDecorView()
                                .performHapticFeedback(HapticFeedbackConstants.LONG_PRESS));
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        setIntent(intent);
        openIntent(intent);
    }

    /** Passes a VIEW intent's URL to native code, which queues it until the app listens. */
    private void openIntent(Intent intent) {
        if (intent != null
                && Intent.ACTION_VIEW.equals(intent.getAction())
                && intent.getData() != null) {
            nativeOpenUrl(intent.getData().toString());
        }
    }

    private String libraryName() {
        try {
            ActivityInfo info =
                    getPackageManager()
                            .getActivityInfo(
                                    new ComponentName(this, getClass()),
                                    PackageManager.GET_META_DATA);
            if (info.metaData != null) {
                String name = info.metaData.getString(META_DATA_LIB_NAME);
                if (name != null) {
                    return name;
                }
            }
        } catch (PackageManager.NameNotFoundException e) {
            Log.e(TAG, "This activity is not in the manifest", e);
        }
        return "main";
    }

    private void sendInsets(WindowInsets insets) {
        if (!nativeReady || insets == null) {
            return;
        }
        Insets bars =
                insets.getInsets(
                        WindowInsets.Type.systemBars() | WindowInsets.Type.displayCutout());
        Insets ime = insets.getInsets(WindowInsets.Type.ime());
        nativeInsets(
                bars.left, bars.top, bars.right, bars.bottom,
                ime.left, ime.top, ime.right, ime.bottom);
    }

    /** Native code is listening: send the current insets. */
    public void attachNative() {
        nativeReady = true;
        runOnUiThread(() -> sendInsets(getWindow().getDecorView().getRootWindowInsets()));
    }

    public void showKeyboard() {
        int request = keyboardRequest.incrementAndGet();
        runOnUiThread(
                () -> {
                    if (request != keyboardRequest.get() || !hasWindowFocus()) {
                        return;
                    }
                    editor.setFocusableInTouchMode(true);
                    editor.requestFocus();
                    // Several logical GPUI fields share this one Android View.
                    // Android can coalesce its blur/refocus and otherwise keep
                    // serving the retired connection from the previous field.
                    if (currentConnection == null || !currentConnection.active) {
                        inputMethods.restartInput(editor);
                    }
                    // Unlike InputMethodManager.showSoftInput, this waits for the input
                    // connection the focus change starts.
                    WindowInsetsController controller = editor.getWindowInsetsController();
                    if (controller != null) {
                        controller.show(WindowInsets.Type.ime());
                    } else {
                        inputMethods.showSoftInput(editor, 0);
                    }
                });
    }

    public void hideKeyboard() {
        int request = keyboardRequest.incrementAndGet();
        runOnUiThread(
                () -> {
                    if (request != keyboardRequest.get()) {
                        return;
                    }
                    WindowInsetsController controller = editor.getWindowInsetsController();
                    if (controller != null) {
                        controller.hide(WindowInsets.Type.ime());
                    } else {
                        inputMethods.hideSoftInputFromWindow(editor.getWindowToken(), 0);
                    }
                    if (currentConnection != null) {
                        currentConnection.retire();
                    }
                    editor.clearFocus();
                    editor.setFocusable(false);
                });
    }

    /** {@link EditorInfo#inputType} and {@link EditorInfo#imeOptions} for the focused input. */
    public void configureKeyboard(int inputType, int imeOptions) {
        runOnUiThread(
                () -> {
                    if (this.inputType == inputType && this.imeOptions == imeOptions) {
                        return;
                    }
                    this.inputType = inputType;
                    this.imeOptions = imeOptions;
                    restartEditor();
                });
    }

    /**
     * The app's text around its selection. Dropped when the keyboard has edited since
     * report {@code basis}; otherwise acknowledged with a report naming {@code pushId}.
     * When the app changed the text or the keyboard's composing word, the keyboard
     * starts over, so it cannot type a composing word the app dropped again. Not
     * when a key the keyboard sent caused it ({@code restart} false), as with an
     * EditText; starting over would cancel a held key.
     */
    public void setText(
            int basis,
            int pushId,
            String text,
            int selectionStart,
            int selectionEnd,
            int composingStart,
            int composingEnd,
            boolean restart) {
        runOnUiThread(
                () -> {
                    if (basis != seq) {
                        return;
                    }
                    // Retired connections can still receive queued keyboard edits.
                    // Invalidate them BEFORE replacing the shared text, so an old
                    // composing word cannot be inserted into a new draft.
                    if (restart && currentConnection != null) {
                        currentConnection.retire();
                    }
                    boolean changed = !text.contentEquals(mirror);
                    if (changed) {
                        mirror.replace(0, mirror.length(), text);
                    }
                    int length = mirror.length();
                    Selection.setSelection(
                            mirror, clamp(selectionStart, length), clamp(selectionEnd, length));
                    BaseInputConnection.removeComposingSpans(mirror);
                    if (0 <= composingStart && composingStart < composingEnd) {
                        mirror.setSpan(
                                composing,
                                clamp(composingStart, length),
                                clamp(composingEnd, length),
                                Spanned.SPAN_EXCLUSIVE_EXCLUSIVE | Spanned.SPAN_COMPOSING);
                    }
                    report(pushId);
                    if (restart) {
                        restartEditor();
                    } else {
                        updateSelection();
                    }
                });
    }

    private Connection currentConnection;

    private void restartEditor() {
        if (currentConnection != null) {
            currentConnection.retire();
        }
        if (editor.hasFocus()) {
            inputMethods.restartInput(editor);
        }
    }

    private static int clamp(int offset, int length) {
        return Math.max(0, Math.min(offset, length));
    }

    private void updateSelection() {
        inputMethods.updateSelection(
                editor,
                Selection.getSelectionStart(mirror),
                Selection.getSelectionEnd(mirror),
                BaseInputConnection.getComposingSpanStart(mirror),
                BaseInputConnection.getComposingSpanEnd(mirror));
    }

    private void report(int pushId) {
        seq++;
        if (!nativeReady) {
            return;
        }
        nativeImeState(
                seq,
                pushId,
                mirror.toString(),
                Selection.getSelectionStart(mirror),
                Selection.getSelectionEnd(mirror),
                BaseInputConnection.getComposingSpanStart(mirror),
                BaseInputConnection.getComposingSpanEnd(mirror));
    }

    /** Reports the mirror once the keyboard's current batch of edits is complete. */
    private void flush() {
        if (batchDepth > 0 || !edited) {
            return;
        }
        edited = false;
        updateSelection();
        report(0);
    }

    // Clipboard methods are safe from any thread. Android only lets the focused app
    // read the clipboard.

    private ClipData clip() {
        ClipboardManager clipboard = getSystemService(ClipboardManager.class);
        return clipboard == null ? null : clipboard.getPrimaryClip();
    }

    /** The clipboard's text, if it has any that is not just an image's URI. */
    public String clipboardText() {
        ClipData clip = clip();
        if (clip == null) {
            return null;
        }
        for (int i = 0; i < clip.getItemCount(); i++) {
            ClipData.Item item = clip.getItemAt(i);
            if (item.getText() != null || item.getHtmlText() != null) {
                return item.coerceToText(this).toString();
            }
            Uri uri = item.getUri();
            String type = uri == null ? null : getContentResolver().getType(uri);
            if (type != null && type.startsWith("text/")) {
                return item.coerceToText(this).toString();
            }
        }
        return null;
    }

    private Uri clipboardImageUri() {
        ClipData clip = clip();
        if (clip == null) {
            return null;
        }
        for (int i = 0; i < clip.getItemCount(); i++) {
            Uri uri = clip.getItemAt(i).getUri();
            String type = uri == null ? null : getContentResolver().getType(uri);
            if (type != null && type.startsWith("image/")) {
                return uri;
            }
        }
        return null;
    }

    /** The MIME type of the clipboard's image, such as {@code image/png}, if it has one. */
    public String clipboardImageType() {
        Uri uri = clipboardImageUri();
        return uri == null ? null : getContentResolver().getType(uri);
    }

    public byte[] clipboardImage() {
        Uri uri = clipboardImageUri();
        if (uri == null) {
            return null;
        }
        try (InputStream in = getContentResolver().openInputStream(uri)) {
            return in == null ? null : Files.read(in);
        } catch (IOException | SecurityException e) {
            Log.w(TAG, "Could not read the copied image", e);
            return null;
        }
    }

    public void setClipboardText(String text) {
        ClipboardManager clipboard = getSystemService(ClipboardManager.class);
        if (clipboard != null) {
            clipboard.setPrimaryClip(ClipData.newPlainText(null, text));
        }
    }

    /** Copies an image, with optional text, served by {@link ClipboardProvider}. */
    public void setClipboardImage(byte[] bytes, String type, String extension, String text) {
        ClipboardManager clipboard = getSystemService(ClipboardManager.class);
        if (clipboard == null) {
            return;
        }
        File folder = ClipboardProvider.folder(this);
        File[] old = folder.listFiles();
        if (old != null) {
            for (File file : old) {
                file.delete();
            }
        }
        folder.mkdirs();
        // A new name each time, so an app still reading the previous image keeps it.
        File file = new File(folder, "image-" + System.currentTimeMillis() + "." + extension);
        try (FileOutputStream out = new FileOutputStream(file)) {
            out.write(bytes);
        } catch (IOException e) {
            Log.w(TAG, "Could not copy the image", e);
            return;
        }
        String[] types = text == null ? new String[] {type} : new String[] {type, "text/plain"};
        ClipData clip =
                new ClipData(
                        "image", types, new ClipData.Item(text, null, ClipboardProvider.uriFor(this, file)));
        clipboard.setPrimaryClip(clip);
    }

    /** Shows the system file picker for documents to open. */
    public void pickFiles(int request, boolean multiple) {
        runOnUiThread(
                () ->
                        startPick(
                                request,
                                false,
                                new Intent(Intent.ACTION_OPEN_DOCUMENT)
                                        .addCategory(Intent.CATEGORY_OPENABLE)
                                        .setType("*/*")
                                        .putExtra(Intent.EXTRA_ALLOW_MULTIPLE, multiple)));
    }

    /** Shows the system file picker for a new document to save to. */
    public void pickSaveFile(int request, String suggestedName) {
        String extension = MimeTypeMap.getFileExtensionFromUrl(suggestedName);
        String type =
                extension == null
                        ? null
                        : MimeTypeMap.getSingleton().getMimeTypeFromExtension(extension);
        runOnUiThread(
                () ->
                        startPick(
                                request,
                                true,
                                new Intent(Intent.ACTION_CREATE_DOCUMENT)
                                        .addCategory(Intent.CATEGORY_OPENABLE)
                                        .setType(type != null ? type : "application/octet-stream")
                                        .putExtra(Intent.EXTRA_TITLE, suggestedName)));
    }

    private void startPick(int request, boolean save, Intent intent) {
        try {
            picks.put(request, save);
            startActivityForResult(intent, request);
        } catch (ActivityNotFoundException e) {
            picks.remove(request);
            picked(request, null, "No app on this device can pick files");
        }
    }

    @Override
    protected void onActivityResult(int request, int result, Intent data) {
        Boolean save = picks.remove(request);
        if (save == null) {
            super.onActivityResult(request, result, data);
            return;
        }
        List<Uri> documents = new ArrayList<>();
        if (result == RESULT_OK && data != null) {
            ClipData several = data.getClipData();
            if (several != null) {
                for (int i = 0; i < several.getItemCount(); i++) {
                    documents.add(several.getItemAt(i).getUri());
                }
            } else if (data.getData() != null) {
                documents.add(data.getData());
            }
        }
        if (documents.isEmpty()) {
            picked(request, null, null);
            return;
        }
        // Copying can take a while for large or remote documents.
        new Thread(
                        () -> {
                            try {
                                List<String> paths =
                                        save
                                                ? List.of(Files.prepareSave(this, request, documents.get(0)))
                                                : Files.copyIn(this, request, documents);
                                picked(request, String.join("\0", paths), null);
                            } catch (IOException | RuntimeException e) {
                                Log.w(TAG, "Could not use the picked files", e);
                                picked(request, null, e.getMessage() != null ? e.getMessage() : e.toString());
                            }
                        },
                        "GpuiFiles")
                .start();
    }

    private void picked(int request, String paths, String error) {
        if (nativeReady) {
            nativePicked(request, paths, error);
        }
    }

    public void openUrl(String url) {
        runOnUiThread(
                () -> {
                    try {
                        startActivity(new Intent(Intent.ACTION_VIEW, Uri.parse(url)));
                    } catch (ActivityNotFoundException e) {
                        Log.w(TAG, "No app can open " + url, e);
                    }
                });
    }

    public boolean notificationsEnabled() {
        return Notifications.enabled(this);
    }

    public void requestNotifications() {
        runOnUiThread(() -> Notifications.requestPermission(this));
    }

    /** See {@link Notifications#post}. */
    public void postNotification(
            String channel,
            String channelName,
            int importance,
            int id,
            String title,
            String text,
            String subtext,
            String url,
            String[] actionLabels,
            String[] actionUrls,
            boolean ongoing,
            int color) {
        Notifications.post(
                this, channel, channelName, importance, id, title, text, subtext, url,
                actionLabels, actionUrls, ongoing, color);
    }

    public void cancelNotification(int id) {
        Notifications.cancel(this, id);
    }

    /** The fastest refresh rate the display offers at its current resolution. */
    public float maxRefreshRate() {
        Display display = getDisplay();
        if (display == null) {
            return 0f;
        }
        Display.Mode current = display.getMode();
        float fastest = current.getRefreshRate();
        for (Display.Mode mode : display.getSupportedModes()) {
            if (mode.getPhysicalWidth() == current.getPhysicalWidth()
                    && mode.getPhysicalHeight() == current.getPhysicalHeight()) {
                fastest = Math.max(fastest, mode.getRefreshRate());
            }
        }
        return fastest;
    }

    /** An invisible view that holds keyboard focus for the native window. */
    private final class EditorView extends View {
        EditorView() {
            super(GpuiActivity.this);
            // Only explicitly activated GPUI fields should own an IME editor.
            setFocusable(false);
        }

        @Override
        public boolean onCheckIsTextEditor() {
            return true;
        }

        @Override
        public InputConnection onCreateInputConnection(EditorInfo info) {
            info.inputType = inputType;
            info.imeOptions =
                    imeOptions
                            | EditorInfo.IME_FLAG_NO_FULLSCREEN
                            | EditorInfo.IME_FLAG_NO_EXTRACT_UI;
            info.initialSelStart = Selection.getSelectionStart(mirror);
            info.initialSelEnd = Selection.getSelectionEnd(mirror);
            info.setInitialSurroundingText(mirror);
            if (currentConnection != null) {
                currentConnection.retire();
            }
            currentConnection = new Connection(this);
            return currentConnection;
        }
    }

    /** Lets {@link BaseInputConnection} edit the mirror, and reports each batch. */
    private final class Connection extends BaseInputConnection {
        private boolean active = true;

        void retire() {
            active = false;
            batchDepth = 0;
            edited = false;
        }

        Connection(View view) {
            super(view, true);
        }

        @Override
        public Editable getEditable() {
            return active ? mirror : null;
        }

        @Override
        public boolean beginBatchEdit() {
            if (!active) return false;
            batchDepth++;
            return true;
        }

        @Override
        public boolean endBatchEdit() {
            if (!active) return false;
            if (batchDepth > 0) {
                batchDepth--;
            }
            flush();
            return batchDepth > 0;
        }

        @Override
        public void closeConnection() {
            if (!active) return;
            super.closeConnection();
            batchDepth = 0;
            flush();
            active = false;
        }

        private boolean edit(boolean result) {
            edited = true;
            flush();
            return result;
        }

        @Override
        public boolean commitText(CharSequence text, int newCursorPosition) {
            if (!active) return false;
            return edit(super.commitText(text, newCursorPosition));
        }

        @Override
        public boolean setComposingText(CharSequence text, int newCursorPosition) {
            if (!active) return false;
            return edit(super.setComposingText(text, newCursorPosition));
        }

        @Override
        public boolean setComposingRegion(int start, int end) {
            if (!active) return false;
            return edit(super.setComposingRegion(start, end));
        }

        @Override
        public boolean finishComposingText() {
            if (!active) return false;
            return edit(super.finishComposingText());
        }

        @Override
        public boolean deleteSurroundingText(int beforeLength, int afterLength) {
            if (!active) return false;
            return edit(super.deleteSurroundingText(beforeLength, afterLength));
        }

        @Override
        public boolean deleteSurroundingTextInCodePoints(int beforeLength, int afterLength) {
            if (!active) return false;
            return edit(super.deleteSurroundingTextInCodePoints(beforeLength, afterLength));
        }

        @Override
        public boolean setSelection(int start, int end) {
            if (!active) return false;
            return edit(super.setSelection(start, end));
        }

        @Override
        public boolean performEditorAction(int action) {
            if (!active) return false;
            if (nativeReady) {
                nativeEditorAction(action);
            }
            return true;
        }
    }
}
