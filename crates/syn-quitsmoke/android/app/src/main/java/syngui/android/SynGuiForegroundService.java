package syngui.android;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Context;
import android.content.Intent;
import android.os.Build;
import android.os.Handler;
import android.os.IBinder;
import android.os.Looper;

/**
 * Foreground-сервис живого прогресс-бара обратного отсчёта («до следующей
 * сигареты»). Сервис сам обновляет уведомление раз в секунду и продолжает
 * работать, когда UI-процесс свёрнут или выгружен.
 *
 * Управляется через {@link SynGuiNotificationHandler} (startForeground/update/stop)
 * по строковому имени класса, поэтому корректно резолвится базовым ClassLoader'ом
 * APK, а не in-memory DEX фреймворка.
 *
 * Extras Intent:
 *   action  : "start" | "update" | "stop"
 *   channel : id канала
 *   title   : заголовок уведомления
 *   start   : long, момент начала отсчёта (epoch ms)
 *   deadline: long, момент «можно курить» (epoch ms)
 *   done_text/ready_text: подписи
 */
public class SynGuiForegroundService extends Service {

    public static final int NOTIF_ID = 0x51530001;

    private final Handler handler = new Handler(Looper.getMainLooper());
    private Runnable ticker;
    private String channelId = "gradual_timer";
    private String title = "";
    private String waitFmt = "%s";
    private String readyText = "Ready";
    private long startMs = 0L;
    private long deadlineMs = 0L;
    private boolean active = false;

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        if (intent == null) {
            return START_STICKY;
        }
        String action = intent.getStringExtra("action");
        if ("stop".equals(action)) {
            stopTicking();
            active = false;
            stopForegroundCompat();
            stopSelf();
            return START_NOT_STICKY;
        }

        if (intent.hasExtra("channel")) channelId = intent.getStringExtra("channel");
        if (intent.hasExtra("title")) title = intent.getStringExtra("title");
        if (intent.hasExtra("ready_text")) readyText = intent.getStringExtra("ready_text");
        if (intent.hasExtra("wait_fmt")) waitFmt = intent.getStringExtra("wait_fmt");
        startMs = intent.getLongExtra("start", System.currentTimeMillis());
        deadlineMs = intent.getLongExtra("deadline", startMs);

        ensureChannel();
        active = true;

        try {
            if (Build.VERSION.SDK_INT >= 34) {
                startForeground(NOTIF_ID, buildNotification(),
                        android.content.pm.ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE);
            } else {
                startForeground(NOTIF_ID, buildNotification());
            }
        } catch (Throwable t) {
            // На случай отсутствия разрешения FGS — деградируем до обычного уведомления.
            NotificationManager nm = (NotificationManager) getSystemService(NOTIFICATION_SERVICE);
            if (nm != null) nm.notify(NOTIF_ID, buildNotification());
        }

        startTicking();
        return START_STICKY;
    }

    private void ensureChannel() {
        if (Build.VERSION.SDK_INT >= 26) {
            NotificationManager nm = (NotificationManager) getSystemService(NOTIFICATION_SERVICE);
            if (nm != null && nm.getNotificationChannel(channelId) == null) {
                NotificationChannel ch = new NotificationChannel(
                        channelId, "Таймер", NotificationManager.IMPORTANCE_LOW);
                ch.setShowBadge(false);
                nm.createNotificationChannel(ch);
            }
        }
    }

    private Notification buildNotification() {
        long now = System.currentTimeMillis();
        long total = Math.max(1L, deadlineMs - startMs);
        long elapsed = now - startMs;
        if (elapsed < 0) elapsed = 0;
        if (elapsed > total) elapsed = total;
        boolean done = now >= deadlineMs;
        int max = 1000;
        int prog = (int) (elapsed * max / total);

        Notification.Builder b;
        if (Build.VERSION.SDK_INT >= 26) {
            b = new Notification.Builder(this, channelId);
        } else {
            b = new Notification.Builder(this);
        }

        int icon = getApplicationInfo().icon;
        if (icon == 0) icon = android.R.drawable.ic_dialog_info;

        b.setSmallIcon(icon)
                .setContentTitle(title)
                .setOngoing(true)
                .setOnlyAlertOnce(true);

        Intent launch = getPackageManager().getLaunchIntentForPackage(getPackageName());
        if (launch != null) {
            int fl = PendingIntent.FLAG_UPDATE_CURRENT;
            if (Build.VERSION.SDK_INT >= 23) fl |= PendingIntent.FLAG_IMMUTABLE;
            b.setContentIntent(PendingIntent.getActivity(this, 0, launch, fl));
        }

        if (done) {
            b.setContentText(readyText).setProgress(0, 0, false);
        } else {
            long leftSec = (deadlineMs - now) / 1000L;
            b.setContentText(formatLeft(leftSec)).setProgress(max, prog, false);
        }
        return b.build();
    }

    private String formatLeft(long sec) {
        long h = sec / 3600;
        long m = (sec % 3600) / 60;
        long s = sec % 60;
        String hms = (h > 0)
                ? String.format("%d:%02d:%02d", h, m, s)
                : String.format("%02d:%02d", m, s);
        try {
            return String.format(waitFmt, hms);
        } catch (Throwable t) {
            return hms;
        }
    }

    private void startTicking() {
        stopTicking();
        ticker = new Runnable() {
            @Override
            public void run() {
                if (!active) return;
                NotificationManager nm = (NotificationManager) getSystemService(NOTIFICATION_SERVICE);
                if (nm != null) {
                    nm.notify(NOTIF_ID, buildNotification());
                }
                handler.postDelayed(this, 1000L);
            }
        };
        handler.postDelayed(ticker, 1000L);
    }

    private void stopTicking() {
        if (ticker != null) {
            handler.removeCallbacks(ticker);
            ticker = null;
        }
    }

    @SuppressWarnings("deprecation")
    private void stopForegroundCompat() {
        if (Build.VERSION.SDK_INT >= 24) {
            stopForeground(Service.STOP_FOREGROUND_REMOVE);
        } else {
            stopForeground(true);
        }
    }

    @Override
    public void onDestroy() {
        stopTicking();
        active = false;
        super.onDestroy();
    }
}
