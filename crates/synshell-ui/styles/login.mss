/* Экран блокировки и входа — общий вид (synshell_ui::login_screen): оболочка (блокировка, lock-preview)
 * и synlogin greeter. Переменные (--accent, --danger, …) — из темы пользователя (у экрана входа —
 * из /var/lib/synshell/theme.toml). */

/* ─── Экран блокировки ───────────────────────────────────────────────────── */

.lock-scrim { background-color: #00000066; }
.lock-root { padding: 80px 40px 32px 40px; }
.lock-time { font-size: 84px; font-weight: bold; color: #ffffff; }
.lock-date { font-size: 20px; color: #ffffffcc; }
.lock-avatar {
    width: 96px;
    height: 96px;
    border-radius: 48px;
    background-color: var(--accent);
}
.lock-avatar-text { font-size: 42px; font-weight: bold; color: var(--accent-fg); }
.lock-user { font-size: 22px; color: #ffffff; }
.lock-icon { icon-size: 20px; icon-color: #ffffffcc; }
.lock-field {
    width: 300px;
    padding: 8px 14px;
    border-radius: 20px;
    background-color: #ffffff22;
    border-width: 1px;
    border-color: #ffffff44;
    color: #ffffff;
    font-size: 15px;
    caret-color: #ffffff;
}
.lock-error { font-size: 13px; color: var(--danger); }
.lock-layout { font-size: 12px; color: #ffffffaa; }

/* ─── Экран блокировки телефона ─────────────────────────────────────────── */

.lock-phone { padding: 56px 24px 28px 24px; }
.lock-phone-time { font-size: 84px; font-weight: 200; color: #ffffff; text-shadow: 0px 2px 12px #00000070; }
.lock-cover .lock-date { color: #ffffffd8; }
.lock-phone-lock { icon-size: 26px; icon-color: #ffffffc0; }
.lock-hint { font-size: 13px; color: #ffffffb0; }
.lock-note { padding: 12px 14px; border-radius: 18px; background-color: #00000055; }
.lock-note-title { font-size: 14px; font-weight: 600; color: #ffffff; }
.lock-note-body { font-size: 13px; color: #ffffffc0; }
.lock-pin .lock-user { color: #ffffff; }
.lock-dot {
    width: 14px; height: 14px; border-radius: 7px;
    border-width: 2px; border-color: #ffffffa0; background-color: #00000000;
    transition: background-color 120ms ease-out, scale 220ms spring(520, 22);
}
.lock-dot-on { background-color: #ffffff; scale: 1.1; }
.lock-dot-checking { background-color: var(--accent); border-color: var(--accent); animation: lock-pulse 700ms ease-in-out infinite; }
@keyframes lock-pulse { 0% { opacity: 1; } 50% { opacity: 0.4; } 100% { opacity: 1; } }
.lock-pad { width: 276px; }
.lock-key {
    width: 80px; height: 80px; border-radius: 40px;
    background-color: #ffffff1c;
    transition: background-color 90ms ease-out, scale 200ms spring(520, 24);
}
.lock-key:active { background-color: #ffffff48; scale: 0.92; }
.lock-key-action { background-color: #00000000; }
.lock-key-digit { font-size: 30px; font-weight: 300; color: #ffffff; padding: 12px 0px 0px 0px; }
.lock-key-sub { font-size: 9px; letter-spacing: 2px; color: #ffffffa0; }
.lock-key-icon { icon-size: 28px; icon-color: #ffffff; }
.lock-pill { padding: 8px 18px; border-radius: 999px; background-color: #ffffff1c; }
.lock-pill-text { font-size: 13px; color: #ffffff; }
.lock-text-box { padding: 12px 18px; border-radius: 16px; background-color: #00000055; width: 280px; }
.lock-text-dots { font-size: 18px; color: #ffffff; letter-spacing: 4px; }
/* Подсказка «Пароль» — без разрядки точек. */
.lock-text-hint { font-size: 16px; color: #ffffffa0; }

