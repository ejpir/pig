package dev.pi.gpui;

import android.content.ContentProvider;
import android.content.ContentValues;
import android.content.Context;
import android.database.Cursor;
import android.database.MatrixCursor;
import android.net.Uri;
import android.os.ParcelFileDescriptor;
import android.provider.OpenableColumns;
import android.webkit.MimeTypeMap;
import java.io.File;
import java.io.FileNotFoundException;

/**
 * Serves images the app copies, read-only: Android's clipboard carries images as
 * {@code content://} URIs, and apps that paste read them from here.
 *
 * <p>Declared in the manifest with the authority {@code <package>.gpui.clipboard}, not
 * exported, granting URI permissions to the apps that read the clipboard.
 */
public final class ClipboardProvider extends ContentProvider {
    static Uri uriFor(Context context, File file) {
        return new Uri.Builder()
                .scheme("content")
                .authority(context.getPackageName() + ".gpui.clipboard")
                .appendPath(file.getName())
                .build();
    }

    static File folder(Context context) {
        return new File(context.getCacheDir(), "clipboard");
    }

    private File file(Uri uri) throws FileNotFoundException {
        String name = uri.getLastPathSegment();
        File file = name == null ? null : new File(folder(getContext()), name);
        if (file == null || !file.getParentFile().equals(folder(getContext())) || !file.isFile()) {
            throw new FileNotFoundException(String.valueOf(uri));
        }
        return file;
    }

    @Override
    public boolean onCreate() {
        return true;
    }

    @Override
    public String getType(Uri uri) {
        String name = uri.getLastPathSegment();
        String extension = name == null ? null : MimeTypeMap.getFileExtensionFromUrl(name);
        String type =
                extension == null
                        ? null
                        : MimeTypeMap.getSingleton().getMimeTypeFromExtension(extension);
        return type != null ? type : "application/octet-stream";
    }

    @Override
    public ParcelFileDescriptor openFile(Uri uri, String mode) throws FileNotFoundException {
        if (!"r".equals(mode)) {
            throw new FileNotFoundException("Copied images are read-only");
        }
        return ParcelFileDescriptor.open(file(uri), ParcelFileDescriptor.MODE_READ_ONLY);
    }

    @Override
    public Cursor query(
            Uri uri, String[] projection, String selection, String[] arguments, String sort) {
        File file;
        try {
            file = file(uri);
        } catch (FileNotFoundException e) {
            return null;
        }
        MatrixCursor cursor =
                new MatrixCursor(new String[] {OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE});
        cursor.addRow(new Object[] {file.getName(), file.length()});
        return cursor;
    }

    @Override
    public Uri insert(Uri uri, ContentValues values) {
        return null;
    }

    @Override
    public int delete(Uri uri, String selection, String[] arguments) {
        return 0;
    }

    @Override
    public int update(Uri uri, ContentValues values, String selection, String[] arguments) {
        return 0;
    }
}
