package dev.pi.gpui;

import android.Manifest;
import android.app.Activity;
import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.graphics.Bitmap;
import android.graphics.Canvas;
import android.graphics.Paint;
import android.graphics.Path;
import android.graphics.drawable.Icon;
import android.net.Uri;
import android.os.Build;
import java.util.ArrayList;
import java.util.List;

/**
 * Notifications whose taps and actions open URLs in the app.
 *
 * <p>Every tap starts the activity with {@code ACTION_VIEW} and the URL; the activity is
 * single-task, so a running app receives it through {@code onNewIntent} and passes it to
 * native code. Starting an activity from the lock screen asks to unlock first.
 */
final class Notifications {
    private static final int PERMISSION_REQUEST = 0x6E6F;
    private static Icon icon;

    private Notifications() {}

    static boolean enabled(Activity activity) {
        return activity.getSystemService(NotificationManager.class).areNotificationsEnabled();
    }

    /** Asks for permission to notify (Android 13 and later), unless already granted. */
    static void requestPermission(Activity activity) {
        if (Build.VERSION.SDK_INT >= 33
                && activity.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS)
                        != PackageManager.PERMISSION_GRANTED) {
            activity.requestPermissions(
                    new String[] {Manifest.permission.POST_NOTIFICATIONS}, PERMISSION_REQUEST);
        }
    }

    /**
     * Posts or replaces notification {@code id}. Importance is a {@code NotificationManager}
     * importance; a channel keeps the importance it was first created with.
     *
     * <p>Segments, when there are any, draw a progress bar: see {@link #showProgress}. An
     * ongoing notification that started at {@code started} (milliseconds since 1970, or 0)
     * counts up from then, and {@code promoted} asks Android 16 to show it as a Live Update.
     */
    static void post(
            Activity activity,
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
            int color,
            int[] segmentLengths,
            int[] segmentColors,
            int[] pointPositions,
            int[] pointColors,
            int progress,
            boolean promoted,
            long started) {
        NotificationManager manager = activity.getSystemService(NotificationManager.class);
        manager.createNotificationChannel(
                new NotificationChannel(channel, channelName, importance));
        Notification.Builder builder =
                new Notification.Builder(activity, channel)
                        .setSmallIcon(icon(activity))
                        .setColor(color)
                        .setContentTitle(title)
                        .setContentText(text)
                        .setStyle(new Notification.BigTextStyle().bigText(text))
                        .setContentIntent(open(activity, url))
                        .setAutoCancel(!ongoing)
                        .setOngoing(ongoing)
                        .setOnlyAlertOnce(true)
                        .setShowWhen(!ongoing);
        if (subtext != null) {
            builder.setSubText(subtext);
        }
        for (int i = 0; i < actionLabels.length; i++) {
            builder.addAction(
                    new Notification.Action.Builder(
                                    icon(activity), actionLabels[i], open(activity, actionUrls[i]))
                            .build());
        }
        if (ongoing && started > 0) {
            builder.setWhen(started).setUsesChronometer(true).setShowWhen(true);
        }
        if (segmentLengths.length > 0) {
            showProgress(builder, segmentLengths, segmentColors, pointPositions, pointColors, progress);
        }
        if (promoted) {
            promote(builder);
        }
        manager.notify(id, builder.build());
    }

    /**
     * A bar of coloured segments with points on it, as Android 16 draws a journey: what is past
     * {@code progress} is faded. Earlier versions, and any failure here, get a plain bar. The
     * style is reached by reflection, so the app builds against an older platform.
     */
    private static void showProgress(
            Notification.Builder builder,
            int[] lengths,
            int[] colors,
            int[] positions,
            int[] pointColors,
            int progress) {
        int max = 0;
        for (int length : lengths) {
            max += length;
        }
        if (Build.VERSION.SDK_INT >= 36) {
            try {
                Class<?> style = Class.forName("android.app.Notification$ProgressStyle");
                Class<?> segment = Class.forName("android.app.Notification$ProgressStyle$Segment");
                Class<?> point = Class.forName("android.app.Notification$ProgressStyle$Point");
                Object progressStyle = style.getConstructor().newInstance();
                List<Object> segments = new ArrayList<>();
                for (int i = 0; i < lengths.length; i++) {
                    Object piece = segment.getConstructor(int.class).newInstance(lengths[i]);
                    segment.getMethod("setColor", int.class).invoke(piece, colors[i]);
                    segments.add(piece);
                }
                List<Object> points = new ArrayList<>();
                for (int i = 0; i < positions.length; i++) {
                    Object mark = point.getConstructor(int.class).newInstance(positions[i]);
                    point.getMethod("setColor", int.class).invoke(mark, pointColors[i]);
                    points.add(mark);
                }
                style.getMethod("setProgressSegments", List.class).invoke(progressStyle, segments);
                style.getMethod("setProgressPoints", List.class).invoke(progressStyle, points);
                style.getMethod("setStyledByProgress", boolean.class).invoke(progressStyle, true);
                style.getMethod("setProgress", int.class).invoke(progressStyle, progress);
                builder.setStyle((Notification.Style) progressStyle);
                return;
            } catch (ReflectiveOperationException | RuntimeException | LinkageError e) {
                // The plain bar below still shows how far along the run is.
            }
        }
        builder.setProgress(max, Math.min(progress, max), false);
    }

    /**
     * Asks Android 16 to promote an ongoing notification to a Live Update. Without the method,
     * the extra it sets is asked for directly; earlier versions ignore both.
     */
    private static void promote(Notification.Builder builder) {
        if (Build.VERSION.SDK_INT < 36) {
            return;
        }
        try {
            Notification.Builder.class
                    .getMethod("setRequestPromotedOngoing", boolean.class)
                    .invoke(builder, true);
        } catch (ReflectiveOperationException | RuntimeException | LinkageError e) {
            builder.getExtras().putBoolean("android.requestPromotedOngoing", true);
        }
    }

    static void cancel(Activity activity, int id) {
        activity.getSystemService(NotificationManager.class).cancel(id);
    }

    private static PendingIntent open(Activity activity, String url) {
        Intent intent = new Intent(Intent.ACTION_VIEW, Uri.parse(url), activity, activity.getClass());
        intent.addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP | Intent.FLAG_ACTIVITY_CLEAR_TOP);
        // Intents that differ only in their URL are distinct, so one request code serves all.
        return PendingIntent.getActivity(
                activity, 0, intent, PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
    }

    /**
     * The status bar icon: the app's white silhouette, or a π drawn in white for
     * an app built without a launcher icon.
     */
    private static synchronized Icon icon(Activity activity) {
        if (icon == null) {
            int id =
                    activity.getResources()
                            .getIdentifier("ic_notification", "drawable", activity.getPackageName());
            if (id != 0) {
                icon = Icon.createWithResource(activity, id);
                return icon;
            }
            int size = 96;
            Bitmap bitmap = Bitmap.createBitmap(size, size, Bitmap.Config.ARGB_8888);
            Paint paint = new Paint(Paint.ANTI_ALIAS_FLAG);
            paint.setColor(0xFFFFFFFF);
            paint.setStyle(Paint.Style.STROKE);
            paint.setStrokeWidth(size / 10f);
            paint.setStrokeCap(Paint.Cap.ROUND);
            paint.setStrokeJoin(Paint.Join.ROUND);
            float u = size / 24f;
            Path pi = new Path();
            pi.moveTo(5 * u, 8 * u);
            pi.lineTo(19 * u, 8 * u);
            pi.moveTo(9 * u, 8 * u);
            pi.lineTo(9 * u, 18 * u);
            pi.moveTo(15 * u, 8 * u);
            pi.lineTo(15 * u, 15.5f * u);
            pi.quadTo(15 * u, 18 * u, 17.5f * u, 18 * u);
            new Canvas(bitmap).drawPath(pi, paint);
            icon = Icon.createWithBitmap(bitmap);
        }
        return icon;
    }
}
