package dev.pi.gpui;

import android.content.Context;
import android.security.keystore.KeyGenParameterSpec;
import android.security.keystore.KeyProperties;
import android.util.AtomicFile;
import android.system.Os;
import android.system.OsConstants;
import java.io.FileDescriptor;
import android.util.Base64;
import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.nio.charset.StandardCharsets;
import java.security.KeyStore;
import java.util.UUID;
import javax.crypto.Cipher;
import javax.crypto.KeyGenerator;
import javax.crypto.SecretKey;
import javax.crypto.spec.GCMParameterSpec;
import org.json.JSONObject;

/** Worker-safe, application-context-only OAuth vault. Never logs credentials or exceptions. */
public final class ProviderVault {
    // Covers all handles, including a replacement Activity and concurrent logout/refresh.
    private static final Object LOCK = new Object();
    private static final int LIMIT = 131072;
    private final Context context;
    private final String scope;

    ProviderVault(Context context, boolean synthetic) {
        this.context = context.getApplicationContext();
        this.scope = synthetic ? "synthetic" : "accounts";
    }
    private String id(String provider) {
        if (!provider.equals("anthropic") && !provider.equals("openai-codex"))
            throw new IllegalArgumentException("unsupported provider");
        return provider;
    }
    private AtomicFile file(String provider) throws Exception {
        File root = context.getNoBackupFilesDir();
        syncDirectory(root.getParentFile()); // getNoBackupFilesDir may have just created root
        File parent = new File(root, "local-providers"); ensureDirectory(parent);
        File dir = new File(parent, scope); ensureDirectory(dir);
        return new AtomicFile(new File(dir, id(provider) + ".json"));
    }
    private void syncDirectory(File directory) throws Exception {
        FileDescriptor fd = Os.open(directory.getAbsolutePath(), OsConstants.O_RDONLY | OsConstants.O_CLOEXEC, 0);
        try {
            if (!OsConstants.S_ISDIR(Os.fstat(fd).st_mode)) throw new Exception("vault directory unavailable");
            Os.fsync(fd);
        } finally { Os.close(fd); }
    }
    private void ensureDirectory(File directory) throws Exception {
        if (!directory.isDirectory()) {
            if (!directory.mkdir()) throw new Exception("vault directory unavailable");
        }
        // Retry durability even if mkdir succeeded before a previous fsync failure.
        syncDirectory(directory); syncDirectory(directory.getParentFile());
    }
    private String alias(String provider) { return "pi.local-providers.v1." + scope + "." + id(provider); }
    private KeyStore keys() throws Exception {
        KeyStore keys = KeyStore.getInstance("AndroidKeyStore"); keys.load(null); return keys;
    }
    private JSONObject envelope(String provider) throws Exception {
        AtomicFile file = file(provider);
        // openRead recovers AtomicFile's legacy backup if necessary.
        if (!file.getBaseFile().exists() && !new File(file.getBaseFile() + ".bak").exists())
            return new JSONObject().put("generation", "").put("empty", true);
        try (FileInputStream stream = file.openRead()) {
            ByteArrayOutputStream bytes = new ByteArrayOutputStream(); byte[] buffer = new byte[4096];
            int count;
            while ((count = stream.read(buffer)) != -1) {
                if (bytes.size() + count > LIMIT) throw new Exception("vault record too large");
                bytes.write(buffer, 0, count);
            }
            JSONObject value = new JSONObject(new String(bytes.toByteArray(), StandardCharsets.UTF_8));
            if (value.getInt("version") != 1 || value.getString("generation").length() > 64)
                throw new Exception("invalid vault record");
            return value;
        }
    }
    private byte[] aad(String provider, String generation) {
        return (context.getPackageName() + ":local-providers:v1:" + scope + ":" + id(provider) + ":" + generation).getBytes(StandardCharsets.UTF_8);
    }
    private void commit(String provider, JSONObject envelope) throws Exception {
        byte[] bytes = envelope.toString().getBytes(StandardCharsets.UTF_8);
        if (bytes.length > LIMIT) throw new Exception("vault record too large");
        AtomicFile file = file(provider); FileOutputStream output = null;
        try {
            output = file.startWrite(); output.write(bytes); output.getFD().sync();
            file.finishWrite(output); output = null;
            // AtomicFile's rename API can log rather than throw: don't acknowledge a lost commit.
            if (!envelope.getString("generation").equals(envelope(provider).getString("generation")))
                throw new Exception("vault commit failed");
            syncDirectory(file.getBaseFile().getParentFile());
        } catch (Exception error) {
            if (output != null) file.failWrite(output);
            throw new Exception("vault write failed");
        }
    }
    public String read(String provider) throws Exception {
        synchronized (LOCK) {
            try {
                JSONObject entry = envelope(provider); String generation = entry.getString("generation");
                Object credential = JSONObject.NULL;
                if (!entry.optBoolean("empty", false)) {
                    SecretKey key = (SecretKey) keys().getKey(alias(provider), null);
                    if (key == null) throw new Exception("vault key unavailable");
                    byte[] iv = Base64.decode(entry.getString("iv"), Base64.NO_WRAP);
                    if (iv.length != 12) throw new Exception("invalid vault nonce");
                    Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
                    cipher.init(Cipher.DECRYPT_MODE, key, new GCMParameterSpec(128, iv));
                    cipher.updateAAD(aad(provider, generation));
                    byte[] plain = cipher.doFinal(Base64.decode(entry.getString("ciphertext"), Base64.NO_WRAP));
                    try { credential = new JSONObject(new String(plain, StandardCharsets.UTF_8)); }
                    finally { java.util.Arrays.fill(plain, (byte) 0); }
                }
                return new JSONObject().put("generation", generation).put("credential", credential).toString();
            } catch (Exception error) { throw new Exception("credential storage unavailable"); }
        }
    }
    /** Null means stale operation. Otherwise returns the newly committed generation. */
    public String write(String provider, String expected, String credential) throws Exception {
        synchronized (LOCK) {
            try {
                JSONObject current = envelope(provider);
                if (!current.getString("generation").equals(expected)) return null;
                byte[] plain = credential.getBytes(StandardCharsets.UTF_8);
                if (plain.length > 65536) throw new Exception("credential too large");
                // Validate JSON without normalizing/dropping upstream provider-specific fields.
                new JSONObject(credential);
                KeyStore keys = keys(); SecretKey key = (SecretKey) keys.getKey(alias(provider), null);
                if (key == null) {
                    if (!current.optBoolean("empty", false)) throw new Exception("vault key unavailable");
                    KeyGenerator generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore");
                    generator.init(new KeyGenParameterSpec.Builder(alias(provider), KeyProperties.PURPOSE_ENCRYPT | KeyProperties.PURPOSE_DECRYPT)
                        .setKeySize(256).setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                        .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE).build());
                    key = generator.generateKey();
                }
                String generation = UUID.randomUUID().toString();
                Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding"); cipher.init(Cipher.ENCRYPT_MODE, key);
                cipher.updateAAD(aad(provider, generation));
                byte[] encrypted;
                try { encrypted = cipher.doFinal(plain); }
                finally { java.util.Arrays.fill(plain, (byte) 0); }
                commit(provider, new JSONObject().put("version", 1).put("generation", generation)
                    .put("iv", Base64.encodeToString(cipher.getIV(), Base64.NO_WRAP))
                    .put("ciphertext", Base64.encodeToString(encrypted, Base64.NO_WRAP)));
                return generation;
            } catch (Exception error) { throw new Exception("credential save failed"); }
        }
    }
    public void clear(String provider) throws Exception {
        synchronized (LOCK) {
            // Tombstone prevents late refresh/login from resurrecting an account. It contains
            // no secret and allows explicit logout even if ciphertext/key was damaged.
            commit(provider, new JSONObject().put("version", 1).put("generation", UUID.randomUUID().toString()).put("empty", true));
            keys().deleteEntry(alias(provider));
        }
    }
}
