package dev.pi.gpui;

import android.Manifest;
import android.app.Activity;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.graphics.Canvas;
import android.graphics.Color;
import android.graphics.ImageFormat;
import android.graphics.Matrix;
import android.graphics.Paint;
import android.graphics.RectF;
import android.graphics.SurfaceTexture;
import android.hardware.camera2.CameraAccessException;
import android.hardware.camera2.CameraCaptureSession;
import android.hardware.camera2.CameraCharacteristics;
import android.hardware.camera2.CameraDevice;
import android.hardware.camera2.CameraManager;
import android.hardware.camera2.CaptureRequest;
import android.hardware.camera2.params.StreamConfigurationMap;
import android.media.Image;
import android.media.ImageReader;
import android.net.Uri;
import android.os.Bundle;
import android.os.Handler;
import android.os.HandlerThread;
import android.os.SystemClock;
import android.util.Log;
import android.util.Size;
import android.view.Gravity;
import android.view.Surface;
import android.view.TextureView;
import android.view.View;
import android.view.ViewGroup;
import android.view.Window;
import android.view.WindowManager;
import android.widget.FrameLayout;
import android.widget.TextView;
import java.nio.ByteBuffer;
import java.util.Arrays;
import java.util.Comparator;
import java.util.concurrent.atomic.AtomicBoolean;

/** A small offline Camera2 QR scanner. Frames go only to the bundled Rust decoder. */
public final class PairScannerActivity extends Activity {
    private static final String TAG = "PiPairScanner";
    private static final int CAMERA_PERMISSION = 71;
    private static final long SCAN_INTERVAL_MS = 140;

    private static native String nativeDecodeQr(
            byte[] luminance, int width, int height, int rowStride);

    private TextureView preview;
    private TextView hint;
    private HandlerThread cameraThread;
    private Handler cameraHandler;
    private CameraDevice camera;
    private CameraCaptureSession session;
    private ImageReader images;
    private Size previewSize;
    private long lastScan;
    private final AtomicBoolean completed = new AtomicBoolean();

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        String library = getIntent().getStringExtra("library");
        System.loadLibrary(library == null ? "main" : library);
        requestWindowFeature(Window.FEATURE_NO_TITLE);
        getWindow().setStatusBarColor(Color.BLACK);
        getWindow().setNavigationBarColor(Color.BLACK);
        getWindow().setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_STATE_ALWAYS_HIDDEN);
        buildUi();
    }

    private void buildUi() {
        FrameLayout root = new FrameLayout(this);
        root.setBackgroundColor(Color.BLACK);
        preview = new TextureView(this);
        root.addView(
                preview,
                new FrameLayout.LayoutParams(
                        ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
        root.addView(
                new ScannerOverlay(this),
                new FrameLayout.LayoutParams(
                        ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));

        TextView close = text("×", 34, Color.WHITE);
        close.setGravity(Gravity.CENTER);
        close.setContentDescription("Close scanner");
        close.setOnClickListener(view -> finish());
        FrameLayout.LayoutParams closeLayout = new FrameLayout.LayoutParams(dp(56), dp(56));
        closeLayout.gravity = Gravity.TOP | Gravity.END;
        closeLayout.setMargins(0, dp(24), dp(10), 0);
        root.addView(close, closeLayout);

        TextView title = text("Scan your computer", 22, Color.WHITE);
        title.setGravity(Gravity.CENTER);
        title.setTypeface(title.getTypeface(), android.graphics.Typeface.BOLD);
        FrameLayout.LayoutParams titleLayout = new FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, dp(64));
        titleLayout.gravity = Gravity.TOP;
        titleLayout.setMargins(dp(64), dp(28), dp(64), 0);
        root.addView(title, titleLayout);

        hint = text("Point the camera at the QR code printed by Pi", 15, Color.WHITE);
        hint.setGravity(Gravity.CENTER);
        hint.setPadding(dp(24), dp(12), dp(24), dp(24));
        FrameLayout.LayoutParams hintLayout = new FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT);
        hintLayout.gravity = Gravity.BOTTOM;
        hintLayout.setMargins(dp(20), 0, dp(20), dp(24));
        root.addView(hint, hintLayout);
        setContentView(root);
    }

    private TextView text(String value, int sp, int color) {
        TextView view = new TextView(this);
        view.setText(value);
        view.setTextSize(sp);
        view.setTextColor(color);
        return view;
    }

    private int dp(int value) {
        return Math.round(value * getResources().getDisplayMetrics().density);
    }

    @Override
    protected void onResume() {
        super.onResume();
        startBackground();
        if (checkSelfPermission(Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) {
            startWhenReady();
        } else {
            requestPermissions(new String[] {Manifest.permission.CAMERA}, CAMERA_PERMISSION);
        }
    }

    @Override
    protected void onPause() {
        closeCamera();
        stopBackground();
        super.onPause();
    }

    @Override
    public void onRequestPermissionsResult(int request, String[] permissions, int[] results) {
        super.onRequestPermissionsResult(request, permissions, results);
        if (request != CAMERA_PERMISSION) return;
        if (results.length > 0 && results[0] == PackageManager.PERMISSION_GRANTED) {
            startWhenReady();
        } else {
            hint.setText("Camera access is needed to scan the pairing code");
        }
    }

    private void startBackground() {
        cameraThread = new HandlerThread("PiQrCamera");
        cameraThread.start();
        cameraHandler = new Handler(cameraThread.getLooper());
    }

    private void stopBackground() {
        if (cameraThread == null) return;
        cameraThread.quitSafely();
        try {
            cameraThread.join();
        } catch (InterruptedException ignored) {
            Thread.currentThread().interrupt();
        }
        cameraThread = null;
        cameraHandler = null;
    }

    private void startWhenReady() {
        if (preview.isAvailable()) {
            openCamera();
            return;
        }
        preview.setSurfaceTextureListener(
                new TextureView.SurfaceTextureListener() {
                    @Override
                    public void onSurfaceTextureAvailable(SurfaceTexture surface, int width, int height) {
                        openCamera();
                    }

                    @Override
                    public void onSurfaceTextureSizeChanged(SurfaceTexture surface, int width, int height) {
                        configureTransform(width, height);
                    }

                    @Override
                    public boolean onSurfaceTextureDestroyed(SurfaceTexture surface) {
                        return true;
                    }

                    @Override
                    public void onSurfaceTextureUpdated(SurfaceTexture surface) {}
                });
    }

    @SuppressWarnings("MissingPermission")
    private void openCamera() {
        if (camera != null || cameraHandler == null || completed.get()) return;
        CameraManager manager = getSystemService(CameraManager.class);
        try {
            String selected = null;
            CameraCharacteristics characteristics = null;
            for (String id : manager.getCameraIdList()) {
                CameraCharacteristics candidate = manager.getCameraCharacteristics(id);
                Integer facing = candidate.get(CameraCharacteristics.LENS_FACING);
                if (facing != null && facing == CameraCharacteristics.LENS_FACING_BACK) {
                    selected = id;
                    characteristics = candidate;
                    break;
                }
            }
            if (selected == null || characteristics == null) {
                hint.setText("No rear camera is available");
                return;
            }
            StreamConfigurationMap map =
                    characteristics.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP);
            if (map == null) throw new CameraAccessException(CameraAccessException.CAMERA_ERROR);
            Size[] sizes = map.getOutputSizes(ImageFormat.YUV_420_888);
            if (sizes == null || sizes.length == 0) {
                hint.setText("This camera cannot provide scanner frames");
                return;
            }
            previewSize = chooseSize(sizes);
            images = ImageReader.newInstance(
                    previewSize.getWidth(), previewSize.getHeight(), ImageFormat.YUV_420_888, 2);
            images.setOnImageAvailableListener(this::scan, cameraHandler);
            configureTransform(preview.getWidth(), preview.getHeight());
            manager.openCamera(selected, cameraState, cameraHandler);
        } catch (CameraAccessException | SecurityException error) {
            Log.w(TAG, "Could not open camera", error);
            hint.setText("The camera couldn't be opened");
        }
    }

    private Size chooseSize(Size[] sizes) {
        return Arrays.stream(sizes)
                .filter(size -> size.getWidth() >= 960 && size.getHeight() >= 540)
                .min(Comparator.comparingLong(size ->
                        Math.abs((long) size.getWidth() * size.getHeight() - 1280L * 720L)))
                .orElseGet(() -> Arrays.stream(sizes)
                        .max(Comparator.comparingLong(size -> (long) size.getWidth() * size.getHeight()))
                        .orElse(new Size(1280, 720)));
    }

    private final CameraDevice.StateCallback cameraState = new CameraDevice.StateCallback() {
        @Override
        public void onOpened(CameraDevice opened) {
            camera = opened;
            createSession();
        }

        @Override
        public void onDisconnected(CameraDevice opened) {
            opened.close();
            camera = null;
        }

        @Override
        public void onError(CameraDevice opened, int error) {
            opened.close();
            camera = null;
            runOnUiThread(() -> hint.setText("The camera stopped unexpectedly"));
        }
    };

    private void createSession() {
        SurfaceTexture texture = preview.getSurfaceTexture();
        if (camera == null || texture == null || images == null) return;
        texture.setDefaultBufferSize(previewSize.getWidth(), previewSize.getHeight());
        Surface surface = new Surface(texture);
        try {
            CaptureRequest.Builder request = camera.createCaptureRequest(CameraDevice.TEMPLATE_PREVIEW);
            request.addTarget(surface);
            request.addTarget(images.getSurface());
            request.set(CaptureRequest.CONTROL_AF_MODE,
                    CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_PICTURE);
            request.set(CaptureRequest.CONTROL_AE_MODE, CaptureRequest.CONTROL_AE_MODE_ON);
            camera.createCaptureSession(
                    Arrays.asList(surface, images.getSurface()),
                    new CameraCaptureSession.StateCallback() {
                        @Override
                        public void onConfigured(CameraCaptureSession configured) {
                            if (camera == null) return;
                            session = configured;
                            try {
                                configured.setRepeatingRequest(request.build(), null, cameraHandler);
                            } catch (CameraAccessException error) {
                                Log.w(TAG, "Could not start camera preview", error);
                            }
                        }

                        @Override
                        public void onConfigureFailed(CameraCaptureSession configured) {
                            runOnUiThread(() -> hint.setText("The camera preview couldn't start"));
                        }
                    },
                    cameraHandler);
        } catch (CameraAccessException error) {
            Log.w(TAG, "Could not create camera session", error);
        }
    }

    private void scan(ImageReader reader) {
        try (Image image = reader.acquireLatestImage()) {
            if (image == null || completed.get()) return;
            long now = SystemClock.elapsedRealtime();
            if (now - lastScan < SCAN_INTERVAL_MS) return;
            lastScan = now;
            Image.Plane plane = image.getPlanes()[0];
            ByteBuffer buffer = plane.getBuffer();
            byte[] luminance = new byte[buffer.remaining()];
            buffer.get(luminance);
            String value = nativeDecodeQr(
                    luminance, image.getWidth(), image.getHeight(), plane.getRowStride());
            if (value == null) return;
            if (!value.startsWith("pi://pair/v1#")) {
                runOnUiThread(() -> hint.setText("That isn't a Pi computer pairing code"));
                return;
            }
            if (completed.compareAndSet(false, true)) {
                runOnUiThread(() -> {
                    setResult(RESULT_OK, new Intent().setData(Uri.parse(value)));
                    finish();
                });
            }
        } catch (RuntimeException error) {
            Log.w(TAG, "Could not decode camera frame", error);
        }
    }

    private void configureTransform(int width, int height) {
        if (previewSize == null || width == 0 || height == 0) return;
        int rotation = getDisplay() == null ? Surface.ROTATION_0 : getDisplay().getRotation();
        Matrix matrix = new Matrix();
        RectF view = new RectF(0, 0, width, height);
        RectF buffer = new RectF(0, 0, previewSize.getHeight(), previewSize.getWidth());
        float centerX = view.centerX();
        float centerY = view.centerY();
        if (rotation == Surface.ROTATION_90 || rotation == Surface.ROTATION_270) {
            buffer.offset(centerX - buffer.centerX(), centerY - buffer.centerY());
            matrix.setRectToRect(view, buffer, Matrix.ScaleToFit.FILL);
            float scale = Math.max(
                    (float) height / previewSize.getHeight(),
                    (float) width / previewSize.getWidth());
            matrix.postScale(scale, scale, centerX, centerY);
            matrix.postRotate(90 * (rotation - 2), centerX, centerY);
        } else if (rotation == Surface.ROTATION_180) {
            matrix.postRotate(180, centerX, centerY);
        }
        preview.setTransform(matrix);
    }

    private void closeCamera() {
        if (session != null) session.close();
        if (camera != null) camera.close();
        if (images != null) images.close();
        session = null;
        camera = null;
        images = null;
    }

    private static final class ScannerOverlay extends View {
        private final Paint shade = new Paint();
        private final Paint frame = new Paint();

        ScannerOverlay(Context context) {
            super(context);
            shade.setColor(0x78000000);
            frame.setColor(Color.WHITE);
            frame.setStyle(Paint.Style.STROKE);
            frame.setStrokeWidth(5f * getResources().getDisplayMetrics().density);
            frame.setStrokeCap(Paint.Cap.ROUND);
            setLayerType(View.LAYER_TYPE_SOFTWARE, null);
        }

        @Override
        protected void onDraw(Canvas canvas) {
            super.onDraw(canvas);
            float side = Math.min(getWidth() * 0.74f, getHeight() * 0.48f);
            float left = (getWidth() - side) / 2f;
            float top = (getHeight() - side) / 2f;
            float right = left + side;
            float bottom = top + side;
            canvas.drawRect(0, 0, getWidth(), top, shade);
            canvas.drawRect(0, bottom, getWidth(), getHeight(), shade);
            canvas.drawRect(0, top, left, bottom, shade);
            canvas.drawRect(right, top, getWidth(), bottom, shade);
            float corner = side * 0.13f;
            canvas.drawLine(left, top, left + corner, top, frame);
            canvas.drawLine(left, top, left, top + corner, frame);
            canvas.drawLine(right, top, right - corner, top, frame);
            canvas.drawLine(right, top, right, top + corner, frame);
            canvas.drawLine(left, bottom, left + corner, bottom, frame);
            canvas.drawLine(left, bottom, left, bottom - corner, frame);
            canvas.drawLine(right, bottom, right - corner, bottom, frame);
            canvas.drawLine(right, bottom, right, bottom - corner, frame);
        }
    }
}
