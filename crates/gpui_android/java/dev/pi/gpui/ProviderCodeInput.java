package dev.pi.gpui;

import android.app.AlertDialog;
import android.text.InputFilter;
import android.text.InputType;
import android.view.View;
import android.view.WindowManager;
import android.view.inputmethod.EditorInfo;
import android.widget.EditText;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.atomic.AtomicLong;
import org.json.JSONObject;

/** Ephemeral native code entry. Never routed through URLs, GPUI fields or telemetry. */
final class ProviderCodeInput {
    private final GpuiActivity activity;
    private final ConcurrentHashMap<String, String> results = new ConcurrentHashMap<>();
    private final AtomicLong generation = new AtomicLong();
    private volatile boolean retired;
    private AlertDialog dialog;
    private EditText input;
    private String active;
    ProviderCodeInput(GpuiActivity activity) { this.activity = activity; }

    void show(String request) {
        final long ticket = generation.incrementAndGet();
        activity.runOnUiThread(() -> {
            if (retired || ticket != generation.get()) return;
            if (activity.isFinishing() || activity.isDestroyed()) { results.put(request, "{\"cancelled\":true}"); return; }
            clearCurrent(); results.clear(); active = request;
            final EditText edit = new EditText(activity); input = edit;
            edit.setInputType(InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_PASSWORD | InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS);
            edit.setImeOptions(EditorInfo.IME_ACTION_DONE | EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING);
            edit.setSaveEnabled(false); edit.setImportantForAutofill(View.IMPORTANT_FOR_AUTOFILL_NO_EXCLUDE_DESCENDANTS);
            edit.setFilters(new InputFilter[]{new InputFilter.LengthFilter(16384)});
            edit.setHint("Authorization code (code#state)");
            final AlertDialog created = new AlertDialog.Builder(activity).setTitle("Claude authorization code")
                .setMessage("Paste only the authorization code returned by Claude, not your password.")
                .setView(edit).setNegativeButton("Cancel", (d, which) -> {})
                .setPositiveButton("Continue", (d, which) -> {
                    if (!retired && ticket == generation.get()) {
                        try { results.put(request, new JSONObject().put("code", edit.getText().toString()).toString()); }
                        catch (Exception ignored) { results.put(request, "{\"cancelled\":true}"); }
                    }
                    edit.getText().clear();
                }).create();
            dialog = created;
            created.setOnDismissListener(d -> {
                edit.getText().clear();
                if (!retired && ticket == generation.get()) results.putIfAbsent(request, "{\"cancelled\":true}");
                if (dialog == created) { dialog = null; input = null; active = null; }
            });
            // Screenshot protection is disabled at the user's request; input remains masked.
            try { created.show(); edit.requestFocus(); }
            catch (WindowManager.BadTokenException error) {
                clearCurrent(); if (!retired && ticket == generation.get()) results.put(request, "{\"cancelled\":true}");
            }
        });
    }
    String take(String request) { return results.remove(request); }
    void dismiss(String request) {
        generation.incrementAndGet(); // also rejects a show that has not reached Java UI yet
        activity.runOnUiThread(() -> {
            if (request.equals(active)) clearCurrent();
            results.remove(request);
        });
    }
    private void clearCurrent() {
        if (input != null) input.getText().clear();
        if (dialog != null) { dialog.setOnDismissListener(null); dialog.dismiss(); }
        dialog = null; input = null; active = null;
    }
    void retire() {
        retired = true; generation.incrementAndGet(); clearCurrent(); results.clear();
    }
}
