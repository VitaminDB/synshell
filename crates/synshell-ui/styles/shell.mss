/* ─────────────────────────────────────────────────────────────────────────
 * syndesktop-shell — встроенная тема.
 * Переменные (--bg, --accent, …) приходят из [appearance] config.toml;
 * переопределять правила — в ~/.config/synshell/theme.mss.
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
}
.panel-vertical { padding: 6px 4px; }
/* Прилипшая к краю панель (`defloat`) — часть экрана, а не остров: без
 * рамки, скруглений и тени. Составной селектор сильнее `.panel` тем. */
.panel.panel-defloated {
    border-width: 0px;
    border-radius: 0px;
    box-shadow: none;
}

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

/* Панель как заголовок окна: заголовок, кнопки, глобальное меню */
.applet-window-title { padding: 4px 10px; }
.window-title-icon { width: 20px; height: 20px; }
.window-title-glyph { icon-size: 18px; }
.window-title-text { font-size: 13px; font-weight: bold; line-height: 20px; }
.applet-window-buttons { padding: 0px; }
.window-button {
    padding: 5px 8px;
    border-radius: var(--radius-sm);
    background-color: #00000000;
    transition: background-color 120ms ease-out;
}
.window-button .icon { icon-size: 18px; }
.window-button:hover { background-color: var(--hover); }
.window-button-close:hover { background-color: var(--danger); }
.window-button-close:hover .icon { icon-color: #ffffff; }
.applet-appmenu { padding: 0px; }
.appmenu-item { padding: 4px 9px; border-radius: var(--radius-sm); }
.appmenu-button {
    background-color: #00000000;
    transition: background-color 120ms ease-out;
}
.appmenu-button:hover { background-color: var(--hover); }
.appmenu-open { background-color: var(--accent-soft); }
.appmenu-open:hover { background-color: var(--accent-soft); }
.appmenu-app { font-size: 13px; font-weight: bold; line-height: 20px; }
.appmenu-label { font-size: 13px; line-height: 20px; }
.menu-shortcut { font-size: 12px; color: var(--muted); }

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

/* Карточка, примыкающая к панели: перетекает в неё — угловые скругления у
 * края панели вогнутые (`flow-edge`), рамка у этого края снята. */
.popup-card.popup-flow-top {
    border-top-left-radius: 0px; border-top-right-radius: 0px;
    border-top-width: 0px;
    flow-edge: top; flow-radius: var(--radius); flow-color: var(--panel-bg);
}
.popup-card.popup-flow-bottom {
    border-bottom-left-radius: 0px; border-bottom-right-radius: 0px;
    border-bottom-width: 0px;
    flow-edge: bottom; flow-radius: var(--radius); flow-color: var(--panel-bg);
}
.popup-card.popup-flow-left {
    border-top-left-radius: 0px; border-bottom-left-radius: 0px;
    border-left-width: 0px;
    flow-edge: left; flow-radius: var(--radius); flow-color: var(--panel-bg);
}
.popup-card.popup-flow-right {
    border-top-right-radius: 0px; border-bottom-right-radius: 0px;
    border-right-width: 0px;
    flow-edge: right; flow-radius: var(--radius); flow-color: var(--panel-bg);
}
/* Содержимое всплывающих окон перетекает: высота списка результатов,
 * смена раздела меню запуска. */
.popup-morph { transition: size 320ms spring(420, 40); }

.popup-title { font-size: 15px; font-weight: bold; color: var(--fg); }
.popup-text { font-size: 13px; color: var(--fg); }
.popup-value { font-size: 13px; color: var(--muted); min-width: 40px; }
.popup-big-icon { icon-size: 30px; icon-color: var(--accent); }

/* Окно «Сеть»: Wi-Fi. */
.net-note { font-size: 12px; color: var(--muted); }
.net-section { font-size: 12px; font-weight: bold; color: var(--muted); }
.net-row {
    padding: 6px 8px;
    border-radius: var(--radius-sm);
    background-color: #00000000;
    transition: background-color 100ms ease-out;
}
.net-row:hover { background-color: var(--hover); }
.net-row-static:hover { background-color: #00000000; }
.net-row-connected { background-color: var(--hover); }
.net-icon { icon-size: 22px; icon-color: var(--muted); }
.net-icon-on { icon-color: var(--accent); }
.net-lock { icon-size: 13px; icon-color: var(--muted); }
.net-ssid { font-size: 13px; color: var(--fg); }
.net-hint { font-size: 11px; color: var(--muted); }
.net-refresh { padding: 4px; border-radius: var(--radius-sm); background-color: #00000000; }
.net-refresh:hover { background-color: var(--hover); }
.net-refresh-icon { icon-size: 18px; icon-color: var(--muted); }
.net-actions { padding: 2px 8px 6px 40px; }
.net-btn { padding: 5px 12px; border-radius: var(--radius-sm); background-color: var(--hover); }
.net-btn:hover { background-color: var(--accent); }
.net-btn:hover .net-btn-label { color: var(--accent-fg); }
.net-btn-label { font-size: 12px; color: var(--fg); }
.net-pass { font-size: 13px; }
.net-toast { font-size: 12px; color: var(--muted); }
.net-status { font-size: 13px; color: var(--fg); }
.net-ok { font-size: 13px; color: var(--success); }
.net-ok-icon { icon-size: 20px; icon-color: var(--success); }
.net-error { font-size: 13px; color: var(--danger); }
.net-error-icon { icon-size: 20px; icon-color: var(--danger); }
.net-spinner { width: 18px; height: 18px; color: var(--accent); }
.net-btn-primary { background-color: var(--accent); }
.net-btn-primary-label { font-size: 12px; color: var(--accent-fg); }
.net-toast-none { height: 0px; }
.net-list { background-color: #00000000; }

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

/* Полоски сигнала мобильной связи (modem::signal_bars) */
.sigbar { width: 3px; border-radius: 1px; }
.sigbar-1 { height: 4px; }
.sigbar-2 { height: 7px; }
.sigbar-3 { height: 10px; }
.sigbar-4 { height: 13px; }
.sigbar-on { background-color: var(--fg); }
.sigbar-off { background-color: var(--pressed); }

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
.osd-busy-icon { icon-size: 20px; icon-color: #3DDC84; }
.osd-value { font-size: 14px; font-weight: bold; min-width: 34px; }
.osd-label { font-size: 14px; }

/* Кнопка «повернуть» при зафиксированной ориентации (как в Android). */
.rotate-suggest {
    width: 100%;
    height: 100%;
    border-radius: 999px;
    background-color: var(--accent);
    border-width: 2px;
    border-color: #ffffff50;
}
.rotate-suggest:active { scale: 0.92; }
.rotate-suggest-icon { icon-size: 36px; icon-color: var(--accent-fg); }

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

/* ─── «Недавние» (телефон) ─────────────────────────────────────────────── */

.recents { padding: 48px 0px 36px 0px; background-color: #000000b0; }
.recents-title { font-size: 20px; font-weight: 600; color: #ffffff; }
.recents-empty { font-size: 15px; color: #ffffffa0; }
.recents-row { padding: 0px 40px; }
.recents-card {
    width: 250px;
    padding: 12px;
    border-radius: 26px;
    background-color: var(--surface);
    box-shadow: 0 14px 40px #00000080;
    transition: scale 220ms spring(420, 26);
}
.recents-card:active { scale: 0.96; }
.recents-app-icon { width: 28px; height: 28px; }
.recents-app-name { font-size: 14px; font-weight: 600; color: var(--fg); }
.recents-preview { height: 420px; width: 226px; border-radius: 18px; background-color: var(--bg); padding: 16px; }
.recents-big-icon { width: 72px; height: 72px; }
.recents-window-title { font-size: 13px; color: var(--muted); text-align: center; }
.recents-clear { padding: 10px 20px; border-radius: 999px; background-color: #ffffff22; }
.recents-clear-icon { icon-size: 20px; icon-color: #ffffff; }
.recents-clear-text { font-size: 14px; color: #ffffff; }

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
.lock-osk { padding: 8px; border-radius: 18px; background-color: #000000a0; }

/* ─── «Пуск» в духе Windows 11 (style = "win11") ──────────────────────────── */

.start { padding: 6px 4px 0px 4px; }
.start-search {
    padding: 9px 16px;
    border-radius: 999px;
    background-color: var(--surface-alt);
    border-width: 1px;
    border-color: var(--border);
    transition: border-color 160ms ease-out;
}
.start-search-icon { icon-size: 20px; icon-color: var(--muted); }
.start-search-field { background-color: #00000000; border-width: 0px; font-size: 15px; color: var(--fg); caret-color: var(--accent); }
.start-body { flex-grow: 1; }
.start-scroll { flex-grow: 1; }
.start-heading { font-size: 14px; font-weight: 600; color: var(--fg); padding: 4px 6px; }
.start-caption { font-size: 12px; color: var(--muted); padding: 2px 8px 6px 8px; }
.start-empty { font-size: 13px; color: var(--muted); padding: 12px 8px; }

.start-pill {
    padding: 4px 10px 4px 12px;
    border-radius: 999px;
    background-color: var(--surface-alt);
    transition: background-color 120ms ease-out, scale 200ms spring(420, 26);
}
.start-pill:hover { background-color: var(--hover); }
.start-pill:active { scale: 0.95; }
.start-pill-text { font-size: 12px; color: var(--fg); }
.start-pill-icon { icon-size: 16px; icon-color: var(--fg); }

/* Источники меню: Linux / Android (по экземпляру) */
.start-chips { min-height: 34px; }
.start-chip {
    padding: 6px 14px 6px 10px;
    border-radius: 999px;
    background-color: var(--surface-alt);
    border-width: 1px;
    border-color: #00000000;
    transition: background-color 120ms ease-out, border-color 120ms ease-out, scale 200ms spring(420, 26);
}
.start-chip:hover { background-color: var(--hover); }
.start-chip:active { scale: 0.95; }
.start-chip-on { background-color: var(--accent); }
.start-chip-on:hover { background-color: var(--accent); }
.start-chip-text { font-size: 13px; color: var(--fg); }
.start-chip-icon { icon-size: 18px; icon-color: var(--fg); }
.start-chip-on .start-chip-text { color: var(--accent-fg); }
.start-chip-on .start-chip-icon { icon-color: var(--accent-fg); }
/* Android — фирменный зелёный значок и рамка (выбранный — как все, в цвете акцента) */
.start-chip-android { border-color: #3DDC8466; }
.start-chip-android .start-chip-icon { icon-color: #3DDC84; }
.start-chip-on .start-chip-icon { icon-color: var(--accent-fg); }
.start-android-badge { padding: 2px; border-radius: 999px; background-color: #3DDC84; }
.start-android-badge-icon { icon-size: 12px; icon-color: #10251a; }

.start-pinned { height: 300px; accent-color: var(--accent); border-color: var(--border); }
.start-tile {
    padding: 10px 2px 8px 2px;
    border-radius: var(--radius-sm);
    background-color: #00000000;
    transition: background-color 120ms ease-out, scale 220ms spring(420, 26);
}
.start-tile:hover { background-color: var(--hover); }
.start-tile:active { scale: 0.93; background-color: var(--pressed); }
.start-tile-icon { width: 36px; height: 36px; }
.start-tile-name { font-size: 12px; color: var(--fg); text-align: center; }

.start-row {
    padding: 7px 10px;
    border-radius: var(--radius-sm);
    background-color: #00000000;
    transition: background-color 120ms ease-out;
}
.start-row:hover { background-color: var(--hover); }
.start-row-body { padding: 7px 10px; }
.start-row-selected { background-color: var(--accent-soft); }
.start-row-icon { width: 32px; height: 32px; }
.start-row-icon-glyph { icon-size: 26px; icon-color: var(--accent); }
.start-row-name { font-size: 14px; color: var(--fg); }
.start-row-sub { font-size: 12px; color: var(--muted); }

.start-letter { font-size: 13px; font-weight: 700; color: var(--accent); padding: 12px 10px 4px 10px; }
.start-rail { padding: 4px 2px; border-radius: 999px; background-color: var(--surface-alt); }
.start-rail-letter {
    height: 18px; width: 22px;
    font-size: 11px; font-weight: 600; text-align: center;
    color: var(--muted);
    border-radius: 9px;
    transition: color 120ms ease-out, background-color 120ms ease-out, scale 180ms spring(500, 24);
}
.start-rail-letter-on { color: var(--accent-fg); background-color: var(--accent); scale: 1.35; }

.start-item-menu { padding: 8px; }
.start-menu-scrim { background-color: #00000018; }

.start-footer {
    padding: 10px 6px 6px 6px;
    border-top-width: 1px;
    border-color: var(--border);
}
.start-avatar { width: 36px; height: 36px; border-radius: 18px; background-color: var(--accent); }
.start-avatar-letter { font-size: 16px; font-weight: 700; color: var(--accent-fg); text-align: center; padding: 7px 0px; }
.start-user { font-size: 14px; font-weight: 600; color: var(--fg); }
.start-footer-btn {
    padding: 8px;
    border-radius: 999px;
    background-color: #00000000;
    transition: background-color 120ms ease-out, scale 200ms spring(420, 26);
}
.start-footer-btn:hover { background-color: var(--hover); }
.start-footer-btn:active { scale: 0.9; }
.start-footer-icon { icon-size: 20px; icon-color: var(--fg); }

/* ─── Шторка (телефон) ─────────────────────────────────────────────────── */

.shade-scrim { background-color: var(--scrim); }
.shade {
    padding: 18px 14px 10px 14px;
    background-color: var(--menu-bg);
    border-bottom-left-radius: 28px;
    border-bottom-right-radius: 28px;
    box-shadow: 0 12px 36px var(--shadow);
}
.shade-time { font-size: 38px; font-weight: 300; color: var(--fg); }
.shade-date { font-size: 14px; color: var(--muted); }
.shade-head-btn {
    padding: 10px;
    border-radius: 999px;
    background-color: var(--surface-alt);
    transition: background-color 120ms ease-out, scale 200ms spring(420, 26);
}
.shade-head-btn:active { scale: 0.9; }
.shade-head-icon { icon-size: 22px; icon-color: var(--fg); }
/* камера включена (значок «используется»): зелёный, как индикатор записи */
.shade-head-active { background-color: #2e7d32; }

.shade-tile {
    padding: 10px 12px;
    border-radius: 22px;
    background-color: var(--surface-alt);
    transition: background-color 220ms ease-out, scale 220ms spring(420, 24);
}
.shade-tile:active { scale: 0.95; }
.shade-tile-on { background-color: var(--accent); }
.shade-tile-badge { padding: 6px; border-radius: 999px; background-color: #ffffff14; }
.shade-tile-icon { icon-size: 20px; icon-color: var(--fg); }
.shade-tile-on .shade-tile-icon { icon-color: var(--accent-fg); }
.shade-tile-label { font-size: 13px; font-weight: 600; color: var(--fg); }
.shade-tile-on .shade-tile-label { color: var(--accent-fg); }
.shade-tile-state { font-size: 11px; color: var(--muted); }
.shade-tile-on .shade-tile-state { color: var(--accent-fg); opacity: 0.8; }

.shade-slider { padding: 8px 12px; border-radius: 22px; background-color: var(--surface-alt); }
.shade-slider-icon { icon-size: 22px; icon-color: var(--fg); }
.shade-slider-value { font-size: 12px; color: var(--muted); }
/* кружок слева у ползунков шторки (36 px — строки одной высоты); автояркость включена — акцентный с «А» */
.shade-auto { width: 36px; height: 36px; border-radius: 18px; align-items: center; justify-content: center; }
.shade-auto-on { background-color: var(--accent); }
.shade-auto-badge { font-size: 15px; font-weight: bold; color: var(--accent-fg); }
/* фонарик: кнопка выключения (значок на акценте), кнопки оттенка «Т»/«Х» выключенные — обводкой */
.shade-auto-badge-icon { icon-size: 20px; icon-color: var(--accent-fg); }
.shade-auto-off { border: 1px solid var(--muted); }
.shade-auto-off .shade-auto-badge { color: var(--fg); }
.shade-notifications { max-height: 320px; }
.shade-handle { width: 44px; height: 5px; border-radius: 3px; background-color: var(--border); }

/* ─── Уведомления ────────────────────────────────────────────────────────── */

.notif {
    padding: 14px 16px;
    border-radius: 22px;
    /* непрозрачный: всплывает поверх всего, текст под ним не должен просвечивать */
    background-color: var(--surface);
    border-width: 1px;
    border-color: var(--border);
    box-shadow: 0 8px 24px var(--shadow);
}
.notif-popups { padding: 12px; }
.notif-in-center { box-shadow: none; background-color: var(--surface-alt); border-radius: var(--radius); }
.notif-critical { border-color: var(--danger); border-width: 2px; }
.notif-low { opacity: 0.92; }
/* Значок приложения — в плашке акцента */
.notif-plate { padding: 8px; border-radius: 14px; background-color: var(--accent-soft); }
.notif-critical .notif-plate { background-color: var(--danger); }
.notif-image { width: 52px; height: 52px; border-radius: 14px; }
.notif-icon { width: 26px; height: 26px; }
.notif-glyph { icon-size: 26px; icon-color: var(--accent); }
.notif-app { font-size: 12px; font-weight: 600; color: var(--muted); }
.notif-time { font-size: 12px; color: var(--muted); }
.notif-summary { font-size: 15px; font-weight: bold; }
.notif-body { font-size: 13px; color: var(--fg); opacity: 0.85; }
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

/* Заголовок задачи в две строки (опция taskbar title = "wrap"). */
.task-title-wrap { font-size: 11px; line-height: 13px; }

/* ─── Значки запуска, разделы и папки на обычной панели ──────────────────── */

.applet-app { padding: 5px 7px; }
.applet-app-running { border-bottom-width: 2px; border-color: var(--muted); }
.applet-app-active { background-color: var(--accent-soft); border-color: var(--accent); }
.applet-app-launching { animation: applet-launch 0.9s ease-in-out infinite; }
@keyframes applet-launch {
    from { opacity: 1; }
    50% { opacity: 0.45; }
    to { opacity: 1; }
}
.applet-group, .applet-folder { padding: 5px 7px; }

/* ─── Док ────────────────────────────────────────────────────────────────── *
 * Корень: .dock-root.dock-<край>.dock-style-<стиль>.dock-ind-<индикатор>
 *         .dock-hover-<эффект>.dock-launch-<анимация> (+ .dock-hidden,
 *         .dock-editing, .dock-floating).
 * Полоса — .dock-items (Fisheye): magnification, magnification-range,
 * magnification-falloff, magnification-speed; подложка может наклоняться
 * в 3D (background-rotate-x). Значок — .dock-item > .dock-icon.
 * ───────────────────────────────────────────────────────────────────────── */

.dock-root { background-color: #00000000; }
.dock-label-none { background-color: #00000000; }

/* Появление и прятанье полосы. */
.dock-slide {
    background-color: #00000000;
    transition: translate-y 320ms ease-out-cubic, translate-x 320ms ease-out-cubic, opacity 240ms ease-out;
    animation: dock-appear 520ms ease-out-back;
}
@keyframes dock-appear {
    from { opacity: 0; scale: 0.85; }
    to { opacity: 1; scale: 1; }
}
.dock-bottom.dock-hidden .dock-slide { translate-y: 160px; opacity: 0; }
.dock-top.dock-hidden .dock-slide { translate-y: -160px; opacity: 0; }
.dock-left.dock-hidden .dock-slide { translate-x: -160px; opacity: 0; }
.dock-right.dock-hidden .dock-slide { translate-x: 160px; opacity: 0; }

/* Полоса. */
.dock-items {
    padding: 6px 8px;
    gap: 4px;
    border-radius: 20px;
    magnification-falloff: cosine;
    magnification-speed: 16;
}
.dock-left .dock-items, .dock-right .dock-items { padding: 8px 6px; }
.dock-editing .dock-items { magnification: 1; }

/* Стиль «стекло» (по умолчанию). */
.dock-style-glass .dock-items {
    background-color: var(--panel-bg);
    border-width: 1px;
    border-color: #ffffff26;
    box-shadow: 0 10px 34px var(--shadow);
}

/* Стиль «полка»: наклонённая в 3D подложка и отражения значков, как в
 * Mac OS X Leopard / Cairo-Dock. */
.dock-style-shelf .dock-items {
    background: linear-gradient(180deg, #ffffff40, #ffffff12);
    border-width: 1px;
    border-color: #ffffff55;
    border-radius: 6px;
    background-rotate-x: 56deg;
    background-perspective: 420px;
    transform-origin: center bottom;
    box-shadow: 0 6px 18px var(--shadow);
}
.dock-style-shelf.dock-top .dock-items { background-rotate-x: -62deg; transform-origin: center top; }
.dock-style-shelf.dock-left .dock-items { background-rotate-x: 0deg; background-rotate-y: -62deg; transform-origin: left center; }
.dock-style-shelf.dock-right .dock-items { background-rotate-x: 0deg; background-rotate-y: 62deg; transform-origin: right center; }
.dock-style-shelf.dock-bottom .dock-icon { box-reflect: below 3px 0.28 42%; }

/* Стиль «плоский». */
.dock-style-flat .dock-items {
    background-color: var(--panel-bg);
    border-radius: 12px;
}

/* Стиль «неон». */
.dock-style-neon .dock-items {
    background-color: #0b0b16e6;
    border-width: 1px;
    border-color: var(--accent);
    border-radius: 18px;
    glow: 0 0 22px var(--accent);
}
.dock-style-neon .dock-dot { background-color: var(--accent); glow: 0 0 8px var(--accent); }

/* Без подложки — только значки. */
.dock-style-none .dock-items { background-color: #00000000; }

/* Значок. */
.dock-item {
    border-radius: 14px;
    background-color: #00000000;
    transition: translate-y 220ms ease-out-back, translate-x 220ms ease-out-back, rotate-y 700ms ease-out-cubic,
        rotate-x 300ms ease-out-cubic, background-color 150ms ease-out, glow 200ms ease-out;
}
.dock-icon { background-color: #00000000; }
.dock-icon-img-glyph { icon-color: var(--accent); }
.dock-item-open { background-color: var(--hover); }

/* Индикаторы окон. */
.dock-dots { gap: 3px; padding: 1px; }
.dock-dot {
    width: 5px;
    height: 5px;
    border-radius: 3px;
    background-color: var(--fg);
    opacity: 0.75;
}
.dock-dot-active { background-color: var(--accent); opacity: 1; }
.dock-ind-line .dock-dot { width: 16px; height: 3px; border-radius: 2px; }
.dock-left.dock-ind-line .dock-dot, .dock-right.dock-ind-line .dock-dot { width: 3px; height: 16px; }
.dock-ind-glow .dock-item-running { glow: 0 0 14px var(--accent-soft); background-color: #ffffff10; }
.dock-ind-glow .dock-dot { opacity: 0; }
.dock-item-minimized .dock-dot { opacity: 0.4; }

/* Эффекты наведения. */
.dock-bottom.dock-hover-lift .dock-item:hover { translate-y: -8px; }
.dock-top.dock-hover-lift .dock-item:hover { translate-y: 8px; }
.dock-left.dock-hover-lift .dock-item:hover { translate-x: 8px; }
.dock-right.dock-hover-lift .dock-item:hover { translate-x: -8px; }
.dock-hover-tilt .dock-item:hover { rotate-y: 24deg; rotate-x: 10deg; perspective: 220px; }
.dock-hover-spin .dock-item:hover { rotate-y: 360deg; }
.dock-hover-glow .dock-item:hover { glow: 0 0 20px var(--accent); }

/* Запуск: значок «прыгает», пока не появится окно. */
.dock-bottom.dock-launch-bounce .dock-item-launching { animation: dock-bounce-up 0.72s ease-in-out infinite; }
.dock-top.dock-launch-bounce .dock-item-launching { animation: dock-bounce-down 0.72s ease-in-out infinite; }
.dock-left.dock-launch-bounce .dock-item-launching { animation: dock-bounce-right 0.72s ease-in-out infinite; }
.dock-right.dock-launch-bounce .dock-item-launching { animation: dock-bounce-left 0.72s ease-in-out infinite; }
.dock-launch-pulse .dock-item-launching { animation: dock-pulse 0.8s ease-in-out infinite; }
.dock-launch-spin .dock-item-launching { animation: dock-spin 1.1s ease-in-out infinite; }

@keyframes dock-bounce-up {
    from { translate-y: 0px; }
    35% { translate-y: -22px; }
    60% { translate-y: 0px; }
    75% { translate-y: -6px; }
    to { translate-y: 0px; }
}
@keyframes dock-bounce-down {
    from { translate-y: 0px; }
    35% { translate-y: 22px; }
    60% { translate-y: 0px; }
    75% { translate-y: 6px; }
    to { translate-y: 0px; }
}
@keyframes dock-bounce-right {
    from { translate-x: 0px; }
    35% { translate-x: 22px; }
    60% { translate-x: 0px; }
    to { translate-x: 0px; }
}
@keyframes dock-bounce-left {
    from { translate-x: 0px; }
    35% { translate-x: -22px; }
    60% { translate-x: 0px; }
    to { translate-x: 0px; }
}
@keyframes dock-pulse {
    from { scale: 1; opacity: 1; }
    50% { scale: 0.86; opacity: 0.6; }
    to { scale: 1; opacity: 1; }
}
@keyframes dock-spin {
    from { rotate-y: 0deg; }
    to { rotate-y: 360deg; }
}

/* Окно просит внимания. */
.dock-item-urgent { animation: dock-attention 1.2s ease-in-out infinite; }
@keyframes dock-attention {
    from { rotate: 0; }
    10% { rotate: -9; }
    20% { rotate: 8; }
    30% { rotate: -6; }
    40% { rotate: 3; }
    50% { rotate: 0; }
    to { rotate: 0; }
}

/* Частицы: при наведении (.dock-fx-hover-<пресет>) и при запуске
 * (.dock-fx-launch-<пресет>). Свои — particle-* в theme.mss. */
.dock-fx-hover-sparkle { particle-preset: sparkle; particle-hover-rate: 14; }
.dock-fx-hover-magic { particle-preset: magic; particle-hover-rate: 18; }
.dock-fx-hover-embers { particle-preset: embers; particle-hover-rate: 16; }
.dock-fx-hover-bubbles { particle-preset: bubbles; particle-hover-rate: 8; }
.dock-fx-hover-hearts { particle-preset: hearts; particle-hover-rate: 6; }
.dock-fx-hover-snow { particle-preset: snow; particle-hover-rate: 10; particle-emitter: line top; }
.dock-fx-hover-trail { particle-preset: sparkle; particle-hover-rate: 40; particle-emitter: pointer; particle-lifetime: 0.35s 0.7s; }
.dock-fx-launch-stars { particle-preset: stars; particle-burst: 26; particle-speed: 60 150; }
.dock-fx-launch-sparkle { particle-preset: sparkle; particle-burst: 40; particle-speed: 30 110; }
.dock-fx-launch-confetti { particle-preset: confetti; particle-burst: 60; particle-speed: 120 260; }
.dock-fx-launch-fireworks { particle-preset: fireworks; particle-burst: 70; particle-speed: 80 200; }
.dock-fx-launch-magic { particle-preset: magic; particle-burst: 40; particle-emitter: ring; }
.dock-fx-launch-poof { particle-preset: poof; }
.dock-top .dock-fx-launch, .dock-top .dock-fx-hover { particle-direction: 90deg; }
.dock-left .dock-fx-launch, .dock-left .dock-fx-hover { particle-direction: 0deg; }
.dock-right .dock-fx-launch, .dock-right .dock-fx-hover { particle-direction: 180deg; }

/* Подпись над значком. */
.dock-label {
    padding: 4px 11px;
    border-radius: 9px;
    background-color: var(--menu-bg);
    border-width: 1px;
    border-color: var(--border);
    box-shadow: 0 4px 14px var(--shadow);
    animation: dock-label-in 160ms ease-out;
}
.dock-label-text { font-size: 13px; color: var(--fg); }
@keyframes dock-label-in {
    from { opacity: 0; translate-y: 4px; }
    to { opacity: 1; translate-y: 0px; }
}

/* Разделитель и прочие апплеты на доке. */
.dock-separator { background-color: #00000000; }
.dock-separator-line { width: 1px; height: 36px; background-color: var(--border); }
.dock-separator-line.dock-separator-h { width: 36px; height: 1px; }
.dock-spacer { width: 12px; height: 12px; }
.dock-applet { padding: 0px 4px; }
.dock-placeholder {
    border-width: 1px;
    border-color: var(--border);
    background-color: #ffffff0d;
}
.dock-placeholder-icon { icon-size: 20px; icon-color: var(--muted); }
.dock-placeholder-label { font-size: 10px; color: var(--muted); }

/* Значок раздела без своей картинки — сетка 2×2 (как папка на iOS). */
.group-preview {
    border-radius: 22%;
    background-color: #ffffff1f;
    border-width: 1px;
    border-color: #ffffff2e;
}
.group-preview-empty { background-color: #00000000; }
.group-preview-icon-glyph { icon-color: var(--fg); }

/* ─── Режим редактирования панелей и дока ────────────────────────────────── */

.edit-frame { background-color: #00000000; }
.edit-item {
    background-color: #00000000;
    animation: edit-wiggle 0.34s ease-in-out infinite;
}
@keyframes edit-wiggle {
    from { rotate: -2.2; }
    50% { rotate: 2.2; }
    to { rotate: -2.2; }
}
.edit-remove {
    width: 18px;
    height: 18px;
    padding: 2px;
    border-radius: 9px;
    background-color: var(--danger);
    box-shadow: 0 1px 4px var(--shadow);
}
.edit-remove .icon { icon-size: 14px; icon-color: #ffffff; }
.edit-btn {
    width: 34px;
    height: 34px;
    padding: 7px;
    border-radius: 17px;
    background-color: var(--surface-alt);
    border-width: 1px;
    border-color: var(--border);
    transition: background-color 120ms ease-out;
}
.edit-btn:hover { background-color: var(--accent-soft); }
.edit-add .icon { icon-color: var(--accent); }
.edit-done { background-color: var(--accent); }
.edit-done .icon { icon-color: var(--accent-fg); }
.edit-controls { padding: 0px 6px; }
.panel-editing { border-color: var(--accent); }

/* Окно «Добавить» и формы. */
.add-tabs { padding-bottom: 2px; }
.add-tab {
    padding: 6px 12px;
    border-radius: 14px;
    background-color: var(--surface-alt);
    transition: background-color 120ms ease-out;
}
.add-tab:hover { background-color: var(--hover); }
.add-tab-active { background-color: var(--accent); }
.add-tab-active .add-tab-label { color: var(--accent-fg); }
.add-tab-label { font-size: 13px; }
.add-hint { font-size: 12px; color: var(--muted); }
.add-scroll { flex-grow: 1; }
.add-app-icon { width: 28px; height: 28px; }
.add-app-hint { font-size: 11px; color: var(--muted); }
.add-app-picked { background-color: var(--accent-soft); }
.add-check { icon-size: 18px; icon-color: var(--muted); }
.add-check-on { icon-color: var(--accent); }
.add-applet {
    width: 96px;
    padding: 10px 4px;
    border-radius: var(--radius-sm);
    background-color: var(--surface-alt);
    transition: background-color 120ms ease-out;
}
.add-applet:hover { background-color: var(--accent-soft); }
.add-applet-icon { icon-size: 24px; icon-color: var(--accent); }
.add-applet-label { font-size: 11px; }

.form-label { font-size: 13px; color: var(--muted); min-width: 86px; }
.form-field {
    padding: 6px 10px;
    border-radius: var(--radius-sm);
    background-color: var(--surface-alt);
    border-width: 1px;
    border-color: var(--border);
    font-size: 13px;
}
.form-app-icon { width: 48px; height: 48px; }
.glyph-pick {
    width: 30px;
    height: 30px;
    padding: 5px;
    border-radius: 8px;
    background-color: var(--surface-alt);
}
.glyph-pick:hover { background-color: var(--hover); }
.glyph-pick-on { background-color: var(--accent-soft); border-width: 1px; border-color: var(--accent); }
.glyph-pick .icon { icon-size: 18px; }
.form-buttons { padding-top: 4px; }
.btn {
    padding: 7px 16px;
    border-radius: var(--radius-sm);
    background-color: var(--surface-alt);
    transition: background-color 120ms ease-out;
}
.btn-primary { background-color: var(--accent); }
.btn-primary .btn-label { color: var(--accent-fg); }
.btn-danger .btn-label { color: var(--danger); }
.btn-label { font-size: 13px; font-weight: 600; }

/* ─── Стеки: раздел или папка во всплывающем окне ────────────────────────── */

.stack-title { padding-bottom: 2px; }
.stack-empty { font-size: 13px; color: var(--muted); padding: 12px 4px; }
.stack-scroll { flex-grow: 1; }
.stack-back {
    padding: 4px;
    border-radius: 14px;
    background-color: var(--surface-alt);
}
.stack-back:hover { background-color: var(--hover); }
.stack-foot { padding-top: 4px; }

.stack-grid-item {
    width: 90px;
    padding: 10px 4px 8px 4px;
    border-radius: var(--radius);
    background-color: #00000000;
    transition: background-color 120ms ease-out, scale 160ms ease-out-back;
}
.stack-grid-item:hover { background-color: var(--hover); scale: 1.06; }
.stack-grid-label { font-size: 11px; text-align: center; }

.stack-list-item {
    padding: 6px 8px;
    border-radius: var(--radius-sm);
    background-color: #00000000;
    transition: background-color 100ms ease-out;
}
.stack-list-item:hover { background-color: var(--hover); }
.stack-list-icon { width: 28px; height: 28px; }
.stack-list-label { font-size: 13px; }

/* Веер: без карточки, подписи в «таблетках». */
.popup-card.popup-card-fan {
    background-color: #00000000;
    border-width: 0px;
    box-shadow: 0 0 0 #00000000;
    padding: 0px;
}
.stack-fan-icon-box { padding: 2px; background-color: #00000000; }
.stack-fan { padding: 4px; }
.stack-fan-item {
    padding: 2px;
    background-color: #00000000;
    transition: scale 140ms ease-out-back;
}
.stack-fan-item:hover { scale: 1.08; }
.stack-fan-label-box {
    padding: 3px 10px;
    border-radius: 10px;
    background-color: var(--menu-bg);
    box-shadow: 0 2px 8px var(--shadow);
}
.stack-fan-label { font-size: 12px; }
.stack-fan-icon { width: 44px; height: 44px; }

/* Шторка: открытые приложения */
.shade-section { font-size: 13px; font-weight: 600; color: var(--muted); }
.shade-apps-all { padding: 4px 10px; border-radius: 999px; }
.shade-apps-all:active { background-color: var(--pressed); }
.shade-apps-all-text { font-size: 13px; color: var(--accent); }
.shade-app {
    padding: 6px 6px 6px 10px;
    border-radius: 18px;
    background-color: var(--surface-alt);
    transition: scale 200ms spring(420, 26);
}
.shade-app:active { scale: 0.95; }
.shade-app-focused { border: 1px solid var(--accent); }
.shade-app-icon { width: 24px; height: 24px; }
.shade-app-name { font-size: 13px; color: var(--fg); max-width: 140px; }
.shade-app-close { padding: 4px; border-radius: 999px; }
.shade-app-close:active { background-color: var(--pressed); }
.shade-app-close-icon { icon-size: 16px; icon-color: var(--muted); }
.recents-empty-box { height: 200px; }
.recents-scroll { height: 500px; width: 100%; }

/* ─── Устройства (synlink): окно, карточка шторки, диалог спаривания ─────── */

.link-card {
    padding: 12px;
    border-radius: 20px;
    background-color: var(--surface-alt);
    border-width: 1px;
    border-color: var(--border);
    transition: background-color 240ms ease-out, border-color 240ms ease-out;
}
.link-card-on {
    background-color: var(--accent-soft);
    border-color: var(--accent-soft);
}
.link-card-slim { padding: 10px 12px; border-radius: 18px; }
.link-card-slim:active { scale: 0.97; }

.link-avatar { padding: 9px; border-radius: 999px; background-color: var(--hover); }
.link-avatar-on {
    background-color: var(--accent);
    box-shadow: 0 4px 16px var(--accent-soft);
}
.link-avatar-big { padding: 20px; }
.location-app-icon { width: 72px; height: 72px; }
.link-avatar-icon { icon-size: 22px; icon-color: var(--fg); }
.link-avatar-on .link-avatar-icon { icon-color: var(--accent-fg); }
.link-avatar-icon-big { icon-size: 44px; }

.link-name { font-size: 14px; font-weight: 600; color: var(--fg); }
.link-state { font-size: 12px; color: var(--muted); }
.link-state-on { color: var(--fg); opacity: 0.75; }

.link-chip { padding: 3px 9px 3px 7px; border-radius: 999px; background-color: var(--hover); }
.link-chip-icon { icon-size: 14px; icon-color: var(--fg); }
.link-chip-text { font-size: 11px; font-weight: 600; color: var(--fg); }
.link-chip-usb { background-color: var(--accent); }
.link-chip-usb .link-chip-icon { icon-color: var(--accent-fg); }
.link-chip-usb .link-chip-text { color: var(--accent-fg); }
.link-chip-wifi { background-color: var(--success); }
.link-chip-wifi .link-chip-icon { icon-color: #ffffff; }
.link-chip-wifi .link-chip-text { color: #ffffff; }

.link-action {
    flex-grow: 1;
    min-width: 62px;
    padding: 9px 4px 7px 4px;
    border-radius: 14px;
    background-color: var(--menu-bg);
    transition: background-color 160ms ease-out, scale 120ms ease-out;
}
.link-action:hover { background-color: var(--hover); }
.link-action:active { scale: 0.94; background-color: var(--pressed); }
.link-action-icon { icon-size: 21px; icon-color: var(--accent); }
.link-action-text { font-size: 11px; color: var(--fg); }

.link-btn { padding: 8px 16px; border-radius: 999px; background-color: var(--hover); transition: scale 120ms ease-out; }
.link-btn:hover { background-color: var(--pressed); }
.link-btn:active { scale: 0.95; }
.link-btn-primary { background-color: var(--accent); }
.link-btn-primary:hover { background-color: var(--accent); opacity: 0.9; }
.link-btn-text { font-size: 13px; font-weight: 600; color: var(--fg); }
.link-btn-primary .link-btn-text { color: var(--accent-fg); }

.link-icon-btn { padding: 5px; border-radius: var(--radius-sm); background-color: #00000000; }
.link-icon-btn:hover { background-color: var(--hover); }
.link-icon-btn-icon { icon-size: 18px; icon-color: var(--muted); }

.link-dot { width: 8px; height: 8px; border-radius: 999px; background-color: var(--muted); }
.link-dot-on { background-color: var(--success); box-shadow: 0 0 8px var(--success); }
.link-dot-wait { background-color: var(--warning); animation: link-pulse 1100ms ease-in-out infinite; }
@keyframes link-pulse { 0% { opacity: 1; } 50% { opacity: 0.25; } 100% { opacity: 1; } }
.link-usb-icon { icon-size: 18px; icon-color: var(--muted); }
.link-usb-icon-on { icon-color: var(--accent); }
.link-usb-text { font-size: 12px; color: var(--fg); }

.link-note { font-size: 12px; color: var(--muted); }
.link-center { text-align: center; }
.link-section { font-size: 12px; font-weight: bold; color: var(--muted); }
.link-empty { padding: 18px 14px; border-radius: 18px; background-color: var(--surface-alt); }
.link-empty-icon { icon-size: 36px; icon-color: var(--muted); }

.link-pair-title { font-size: 18px; font-weight: bold; color: var(--fg); }
.link-pair-text { font-size: 13px; color: var(--muted); text-align: center; }
.link-code-box { padding: 10px 22px; border-radius: 18px; background-color: var(--surface-alt); border-width: 1px; border-color: var(--accent-soft); }
.link-code { font-size: 34px; font-weight: bold; letter-spacing: 4px; font-family: monospace; color: var(--accent); }
.link-shield { icon-size: 14px; icon-color: var(--success); }

.link-applet-on { icon-color: var(--accent); }
.link-applet-usb { icon-size: 14px; icon-color: var(--accent); }
.link-applet-battery { font-size: 11px; color: var(--muted); }
