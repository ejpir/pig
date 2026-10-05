package dev.pi.gpui;

import android.content.Context;
import android.database.Cursor;
import android.net.Uri;
import android.os.FileObserver;
import android.provider.OpenableColumns;
import android.util.Log;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileNotFoundException;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.util.ArrayList;
import java.util.List;

/**
 * Documents from Android's file picker as ordinary files, for native code that works with
 * paths.
 *
 * <p>The picker returns {@code content://} documents, which may live in other apps or the
 * cloud. Opened documents are copied into the app's cache under their own names. A
 * document chosen to save to gets a path in the cache; whenever the app finishes writing
 * that file, it is copied to the document.
 */
final class Files {
    private static final String TAG = "GpuiActivity";
    /** Keeps save watchers alive; they stop when collected. */
    private static final List<FileObserver> SAVES = new ArrayList<>();

    private Files() {}

    /** Forgets files picked in earlier runs; their paths were only good for that run. */
    static void clearPicked(Context context) {
        delete(new File(context.getCacheDir(), "picked"));
        delete(new File(context.getCacheDir(), "saving"));
    }

    private static void delete(File file) {
        File[] children = file.listFiles();
        if (children != null) {
            for (File child : children) {
                delete(child);
            }
        }
        file.delete();
    }

    /** A fresh folder for one pick, so names from different picks never collide. */
    private static File folder(Context context, String kind, int request) throws IOException {
        File folder = new File(context.getCacheDir(), kind + "/" + request + "-" + System.nanoTime());
        if (!folder.mkdirs()) {
            throw new IOException("Cannot create " + folder);
        }
        return folder;
    }

    /** Copies opened documents into the cache; returns their paths. */
    static List<String> copyIn(Context context, int request, List<Uri> documents)
            throws IOException {
        File folder = folder(context, "picked", request);
        List<String> paths = new ArrayList<>();
        for (Uri document : documents) {
            File file = unique(folder, displayName(context, document));
            try (InputStream in = context.getContentResolver().openInputStream(document);
                    OutputStream out = new FileOutputStream(file)) {
                if (in == null) {
                    throw new FileNotFoundException(document.toString());
                }
                copy(in, out);
            }
            paths.add(file.getAbsolutePath());
        }
        return paths;
    }

    /** A path whose writes reach {@code document}. */
    static String prepareSave(Context context, int request, Uri document) throws IOException {
        File folder = folder(context, "saving", request);
        File file = new File(folder, displayName(context, document));
        // Watch the folder rather than the file: apps may write a temporary file and
        // rename it into place.
        FileObserver observer =
                new FileObserver(folder, FileObserver.CLOSE_WRITE | FileObserver.MOVED_TO) {
                    @Override
                    public void onEvent(int event, String name) {
                        if (file.getName().equals(name)) {
                            copyOut(context, file, document);
                        }
                    }
                };
        observer.startWatching();
        synchronized (SAVES) {
            SAVES.add(observer);
        }
        return file.getAbsolutePath();
    }

    private static void copyOut(Context context, File file, Uri document) {
        try (InputStream in = new FileInputStream(file);
                OutputStream out = context.getContentResolver().openOutputStream(document, "wt")) {
            if (out == null) {
                throw new FileNotFoundException(document.toString());
            }
            copy(in, out);
        } catch (IOException | SecurityException e) {
            Log.w(TAG, "Could not save " + file.getName(), e);
        }
    }

    static String displayName(Context context, Uri document) {
        String name = null;
        try (Cursor cursor =
                context.getContentResolver()
                        .query(
                                document,
                                new String[] {OpenableColumns.DISPLAY_NAME},
                                null,
                                null,
                                null)) {
            if (cursor != null && cursor.moveToFirst() && !cursor.isNull(0)) {
                name = cursor.getString(0);
            }
        } catch (RuntimeException e) {
            Log.w(TAG, "No name for " + document, e);
        }
        if (name == null || name.isEmpty()) {
            name = document.getLastPathSegment();
        }
        if (name == null || name.isEmpty()) {
            name = "file";
        }
        // Keep it one path component.
        return name.replace('/', '_').replace('\0', '_');
    }

    /** {@code name} in {@code folder}, numbered if two picked documents share it. */
    private static File unique(File folder, String name) {
        File file = new File(folder, name);
        int dot = name.lastIndexOf('.');
        String stem = dot > 0 ? name.substring(0, dot) : name;
        String extension = dot > 0 ? name.substring(dot) : "";
        for (int n = 2; file.exists(); n++) {
            file = new File(folder, stem + " (" + n + ")" + extension);
        }
        return file;
    }

    static byte[] read(InputStream in) throws IOException {
        java.io.ByteArrayOutputStream out = new java.io.ByteArrayOutputStream();
        copy(in, out);
        return out.toByteArray();
    }

    private static void copy(InputStream in, OutputStream out) throws IOException {
        byte[] buffer = new byte[64 * 1024];
        for (int read; (read = in.read(buffer)) != -1; ) {
            out.write(buffer, 0, read);
        }
    }
}
