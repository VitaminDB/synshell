/* ─────────────────────────────────────────────────────────────────────────
 * syndesktop-shell — встроенная тема.
 * Переменные (--bg, --accent, …) приходят из [appearance] config.toml;
 * переопределять правила — в ~/.config/syndesktop/theme.mss.
 * ───────────────────────────────────────────────────────────────────────── */

Text {
    color: var(--fg);
    font-size: var(--font-size);
}

.grow { flex-grow: 1; }
.muted { color: var(--muted); icon-color: var(--muted); }

.icon {
        icon-size: 20px;
    icon-color: var(--fg);
}

/* ─── Панель ─────────────────────────────────────────────────────────────── */

.panel-root { background-color: #00000000; }
.panel-hidden { background-color: #00000001; }

.panel {
    flex-grow: 1;
    background-color: var(--panel-bg);
    border-width: 1px;
    border-color: var(--border);
    padding: 4px 6px;
}
.panel-floating {
    border-radius: var(--radius);
    box-shadow: 0 4px 16px var(--shadow);
}
.panel-vertical { padding: 6px 4px; }

.panel-content { flex-grow: 1; gap: 4px; }

.applet-slot-spacer { flex-grow: 1; }
.applet-slot-taskbar { flex-grow: 100; }
.applet-spacer { flex-grow: 1; }

.applet {
    padding: 6px 9px;
    border-radius: var(--radius-sm);
    background-color: #00000000;
    transition: background-color 120ms ease-out;
}
.applet:hover { background-color: var(--hover); }

.applet-label { color: var(--fg); font-size: 13px; line-height: 20px; }
.applet-mono { font-size: 12px; }
.applet-image { width: 20px; height: 20px; }
.applet-separator { width: 1px; height: 24px; background-color: var(--border); }
.applet-separator-h { height: 1px; width: 24px; background-color: var(--border); }

.applet-launcher .icon { icon-color: var(--accent); icon-size: 22px; }

.badge {
    background-color: var(--accent);
    border-radius: 8px;
    padding: 0px 5px;
}
.badge-text { color: var(--accent-fg); font-size: 10px; font-weight: bold; }

.keyboard-label { font-weight: bold; font-size: 12px; line-height: 20px; }

.applet-clock { padding: 1px 10px; }
.clock-time { font-size: 14px; font-weight: bold; }
.clock-date { font-size: 10px; color: var(--muted); }
.clock-date-v { font-size: 10px; }

.battery-low .icon { icon-color: var(--danger); }

/* Пейджер столов */
.applet-workspaces { padding: 2px; }
.ws {
    min-width: 26px;
    padding: 4px 8px;
    border-radius: var(--radius-sm);
    background-color: #00000000;
    transition: background-color 120ms ease-out;
}
.ws:hover { background-color: var(--hover); }
.ws-active { background-color: var(--accent-soft); }
.ws-active:hover { background-color: var(--accent-soft); }
.ws-label { font-size: 12px; color: var(--muted); line-height: 20px; }
.ws-active .ws-label { color: var(--fg); font-weight: bold; }
.ws-urgent .ws-label { color: var(--warning); }
.ws-dot { width: 4px; height: 4px; border-radius: 2px; background-color: var(--muted); }
.ws-active .ws-dot { background-color: var(--accent); }

/* Панель задач */
.applet-taskbar { padding: 0px; }
.task {
    padding: 5px 10px;
    border-radius: var(--radius-sm);
    background-color: #00000000;
    border-width: 1px;
    border-color: #00000000;
    transition: background-color 120ms ease-out;
}
.task:hover { background-color: var(--hover); }
.task-active {
    background-color: var(--accent-soft);
    border-color: var(--accent-soft);
}
.task-active:hover { background-color: var(--accent-soft); }
.task-minimized .task-title { color: var(--muted); }
.task-urgent { border-color: var(--warning); }
.task-icon-only { padding: 5px 6px; }
.task-icon { width: 22px; height: 22px; }
.task-glyph { icon-size: 20px; }
.task-title { font-size: 13px; line-height: 22px; }

/* ─── Всплывающие окна ───────────────────────────────────────────────────── */

.popup-backdrop { background-color: #00000000; }

.popup-card {
    background-color: var(--menu-bg);
    border-width: 1px;
    border-color: var(--border);
    border-radius: var(--radius);
    padding: 14px;
    box-shadow: 0 10px 32px var(--shadow);
}

.popup-title { font-size: 15px; font-weight: bold; color: var(--fg); }
.popup-text { font-size: 13px; color: var(--fg); }
.popup-value { font-size: 13px; color: var(--muted); min-width: 40px; }
.popup-big-icon { icon-size: 30px; icon-color: var(--accent); }

.menu-item {
    padding: 8px 10px;
    border-radius: var(--radius-sm);
    background-color: #00000000;
    transition: background-color 100ms ease-out;
}
.menu-item:hover { background-color: var(--hover); }
.menu-icon { icon-size: 18px; icon-color: var(--muted); }
.menu-label { font-size: 13px; }
.menu-sep { height: 1px; min-width: 180px; background-color: var(--border); }

.round-btn {
    width: 34px;
    height: 34px;
    border-radius: 17px;
    padding: 7px;
    background-color: var(--surface-alt);
    transition: background-color 100ms ease-out;
}
.round-btn:hover { background-color: var(--pressed); }

.cal-time { font-size: 26px; font-weight: bold; }
.cal-date { font-size: 13px; color: var(--muted); }

.power-btn {
    width: 62px;
    padding: 10px 4px;
    border-radius: var(--radius-sm);
    background-color: var(--surface-alt);
    transition: background-color 100ms ease-out;
}
.power-btn:hover { background-color: var(--accent-soft); }
.power-icon { icon-size: 26px; icon-color: var(--fg); }
.power-label { font-size: 11px; color: var(--muted); }

.meter { height: 6px; }
.meter-fill { height: 6px; border-radius: 3px; background-color: var(--accent); }
.meter-rest { height: 6px; border-radius: 3px; background-color: var(--pressed); }

.chip {
    padding: 4px 10px;
    border-radius: 14px;
    background-color: var(--surface-alt);
}
.chip-on { background-color: var(--accent-soft); }
.chip-label { font-size: 12px; }

/* ─── OSD ────────────────────────────────────────────────────────────────── */

.osd {
    flex-grow: 1;
    padding: 14px 20px;
    border-radius: 18px;
    background-color: var(--menu-bg);
    border-width: 1px;
    border-color: var(--border);
}
.osd-icon { icon-size: 24px; icon-color: var(--accent); }
.osd-value { font-size: 14px; font-weight: bold; min-width: 34px; }
.osd-label { font-size: 14px; }

/* ─── Меню запуска ───────────────────────────────────────────────────────── */

.search-box {
    padding: 6px 12px;
    border-radius: var(--radius-sm);
    background-color: var(--surface-alt);
    border-width: 1px;
    border-color: var(--border);
}
.search-icon { icon-size: 20px; icon-color: var(--muted); }
.search-field {
    background-color: #00000000;
    border-width: 0px;
    font-size: 15px;
    color: var(--fg);
    caret-color: var(--accent);
}

.launcher-side-wrap { width: 190px; }
.launcher-sidebar { flex-grow: 1; }
.cat-item {
    padding: 8px 10px;
    border-radius: var(--radius-sm);
    background-color: #00000000;
    transition: background-color 100ms ease-out;
}
.cat-item:hover { background-color: var(--hover); }
.cat-item-active { background-color: var(--accent-soft); }
.cat-item-active:hover { background-color: var(--accent-soft); }

.launcher-list { flex-grow: 1; }
.launcher-row {
    padding: 6px 10px;
    border-radius: var(--radius-sm);
    background-color: #00000000;
}
.launcher-row-selected { background-color: var(--accent-soft); }
.launcher-icon { width: 32px; height: 32px; }
.launcher-icon-glyph { icon-size: 28px; icon-color: var(--accent); }
.launcher-name { font-size: 14px; }
.launcher-sub { font-size: 12px; color: var(--muted); }
.launcher-empty { font-size: 13px; color: var(--muted); padding: 16px; }

.launcher-footer {
    padding: 8px 4px 0px 4px;
    border-color: var(--border);
}
.launcher-user { font-size: 14px; font-weight: bold; }
.footer-btn {
    padding: 7px;
    border-radius: var(--radius-sm);
    background-color: #00000000;
    transition: background-color 100ms ease-out;
}
.footer-btn:hover { background-color: var(--hover); }

.launcher-fs-backdrop { background-color: var(--scrim); }
.launcher-fs { padding: 60px 40px; }
.launcher-fs-search { width: 520px; }
.launcher-fs-grid { flex-grow: 1; }
.grid-cell {
    width: 120px;
    padding: 12px 6px;
    border-radius: var(--radius);
    background-color: #00000000;
    transition: background-color 100ms ease-out;
}
.grid-cell:hover { background-color: var(--hover); }
.grid-cell-selected { background-color: var(--accent-soft); }
.grid-icon { width: 56px; height: 56px; }
.grid-icon-glyph { icon-size: 48px; icon-color: var(--accent); }
.grid-name { font-size: 12px; }

/* ─── Уведомления ────────────────────────────────────────────────────────── */

.notif {
    padding: 12px 14px;
    border-radius: var(--radius);
    background-color: var(--menu-bg);
    border-width: 1px;
    border-color: var(--border);
    box-shadow: 0 6px 22px var(--shadow);
}
.notif-in-center { box-shadow: none; background-color: var(--surface-alt); }
.notif-critical { border-color: var(--danger); }
.notif-low { opacity: 0.92; }
.notif-image { width: 48px; height: 48px; border-radius: 8px; }
.notif-icon { width: 36px; height: 36px; }
.notif-glyph { icon-size: 30px; icon-color: var(--accent); }
.notif-app { font-size: 11px; color: var(--muted); }
.notif-time { font-size: 11px; color: var(--muted); }
.notif-summary { font-size: 14px; font-weight: bold; }
.notif-body { font-size: 13px; color: var(--fg); }
.notif-close {
    padding: 2px;
    border-radius: 10px;
    background-color: #00000000;
}
.notif-close .icon { icon-size: 16px; icon-color: var(--muted); }
.notif-close:hover { background-color: var(--hover); }
.notif-action {
    padding: 5px 12px;
    border-radius: var(--radius-sm);
    background-color: var(--surface-alt);
}
.notif-action:hover { background-color: var(--accent-soft); }
.notif-action-label { font-size: 12px; }
.notif-center-list { height: 420px; }

/* ─── Переключатель окон ─────────────────────────────────────────────────── */

.switch-card {
    width: 140px;
    padding: 12px 8px;
    border-radius: var(--radius);
    background-color: #00000000;
    border-width: 2px;
    border-color: #00000000;
}
.switch-card-selected {
    background-color: var(--accent-soft);
    border-color: var(--accent);
}
.switch-icon { width: 48px; height: 48px; }
.switch-title { font-size: 12px; }

/* ─── Обои и значки рабочего стола ───────────────────────────────────────── */

.wallpaper-image { flex-grow: 1; }
.desk-icons { padding: 16px; }
.desk-item {
    width: 96px;
    padding: 8px 4px;
    border-radius: var(--radius-sm);
    background-color: #00000000;
}
.desk-item:hover { background-color: #ffffff22; }
.desk-icon { width: 48px; height: 48px; }
.desk-label { font-size: 12px; color: #ffffff; text-shadow: 0 1px 3px #000000cc; }

.switcher {
    padding: 12px;
    border-radius: var(--radius);
    background-color: var(--menu-bg);
    border-width: 1px;
    border-color: var(--border);
}

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

/* Календарь во всплывающем окне часов */
.cal {
    background-color: var(--menu-bg);
    border-color: var(--menu-bg);
    color: var(--fg);
    accent-color: var(--accent);
    --cal-panel-bg: var(--menu-bg);
    --cal-panel-border: var(--menu-bg);
    --cal-hover-bg: var(--hover);
    --cal-cell-size: 35px;
}

/* Поля ввода оболочки */
TextField {
    color: var(--fg);
    caret-color: var(--accent);
    selection-color: var(--accent-soft);
}

/* ─── Системный лоток ────────────────────────────────────────────────────── */

.applet-tray { padding: 0px; }
.tray-item {
    padding: 5px;
    border-radius: var(--radius-sm);
    background-color: #00000000;
    transition: background-color 120ms ease-out;
}
.tray-item:hover { background-color: var(--hover); }
.tray-attention { background-color: var(--accent-soft); }
.tray-more .icon { icon-size: 16px; icon-color: var(--muted); }
.menu-disabled { opacity: 0.45; }
.menu-img { width: 18px; height: 18px; }
.menu-icon-space { width: 18px; height: 18px; }
