/* Терминал: всё моноширинное, всё светится. */

Text, TextField { font-family: "Hack"; }

.panel { border-color: var(--border); border-radius: 0px; }
.applet, .task, .ws, .tray-item, .menu-item, .cat-item, .launcher-row, .grid-cell, .chip, .round-btn, .power-btn, .badge {
    border-radius: 0px;
}

.applet-label, .task-title, .ws-label { text-shadow: 0 0 4px var(--phosphor-glow); }
.ws-label { text-transform: uppercase; }
.ws-active { background-color: var(--phosphor-soft); border-width: 1px; border-color: var(--phosphor); }
.ws-active .ws-label { color: var(--phosphor); }
.ws-active .ws-dot { background-color: var(--phosphor); }

.task-active { background-color: var(--phosphor-soft); border-color: var(--phosphor); }

.clock-time { color: var(--phosphor); text-shadow: 0 0 8px var(--phosphor-glow); }
.clock-date { color: var(--muted); text-transform: uppercase; }
.keyboard-label { color: var(--amber); }
.applet-launcher .icon { icon-color: var(--phosphor); }
.icon { icon-color: var(--fg); }

.popup-card, .notif, .osd, .switcher {
    border-radius: 0px;
    border-color: var(--phosphor);
    box-shadow: 0 0 14px var(--phosphor-soft), 0 12px 30px var(--shadow);
}
.popup-title, .notif-summary, .launcher-user {
    color: var(--phosphor);
    text-transform: uppercase;
    text-shadow: 0 0 6px var(--phosphor-glow);
}
.notif-app, .notif-time { color: var(--amber); }

.search-box { border-radius: 0px; border-color: var(--phosphor); }
.search-field { caret-color: var(--phosphor); }
.launcher-row-selected, .cat-item-active, .grid-cell-selected {
    background-color: var(--phosphor);
}
.launcher-row-selected .launcher-name, .launcher-row-selected .launcher-sub,
.cat-item-active .menu-label, .grid-cell-selected .grid-name { color: #021005; text-shadow: none; }
.cat-item-active .menu-icon { icon-color: #021005; }
.launcher-icon-glyph, .grid-icon-glyph, .popup-big-icon, .osd-icon { icon-color: var(--phosphor); }

.meter-fill, .meter-rest { border-radius: 0px; }
.meter-fill { glow: 0 0 6px var(--phosphor-glow); }
.badge { background-color: var(--amber); }
.badge-text { color: #021005; }
.notif-image { border-radius: 0px; }
.wallpaper-color { vignette: 0.55; noise: 0.06; }
.lock-time { color: var(--phosphor); text-shadow: 0 0 22px var(--phosphor-glow); }
.lock-avatar { border-radius: 0px; background-color: var(--phosphor); }
.lock-field { border-radius: 0px; border-color: var(--phosphor); }
