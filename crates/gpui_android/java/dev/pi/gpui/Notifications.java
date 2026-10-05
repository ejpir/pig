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
            int color) {
        NotificationManager manager = activity.getSystemService(NotificationManager.class);
        manager.createNotificationChannel(
                new NotificationChannel(channel, channelName, importance));
        Notification.Builder builder =
                new Notification.Builder(activity, channel)
                        .setSmallIcon(icon())
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
                                    icon(), actionLabels[i], open(activity, actionUrls[i]))
                            .build());
        }
        manager.notify(id, builder.build());
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

    /** The status bar icon: a π drawn in white, since the app has no resources. */
    private static synchronized Icon icon() {
        if (icon == null) {
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
