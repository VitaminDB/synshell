/* Аврора: стекло — полупрозрачный фон, светлая кромка сверху. */

.panel {
    background-color: var(--panel-bg);
    border-color: var(--glass-edge);
    box-shadow: inset 0 1px 0 var(--glass-hi);
}
.panel-floating { border-radius: 18px; }

.applet:hover, .task:hover, .ws:hover, .tray-item:hover { background-color: var(--glass-hi); }
.ws, .task, .applet, .tray-item { border-radius: 12px; }

.ws-active {
    background-color: var(--glass-hi);
    border-width: 1px;
    border-color: var(--glass-edge);
}
.ws-active .ws-label { color: var(--accent); }
.ws-active .ws-dot { background-color: var(--accent); }

.task-active {
    background-color: var(--glass-hi);
    border-color: var(--glass-edge);
}

.clock-time { color: var(--fg); }
.clock-date { color: var(--accent); }
.applet-launcher .icon { icon-color: var(--accent); }

.popup-card, .notif, .osd, .switcher {
    background-color: var(--menu-bg);
    border-color: var(--glass-edge);
    border-radius: 20px;
    box-shadow: inset 0 1px 0 var(--glass-hi), 0 20px 50px var(--shadow);
}

.search-box {
    background-color: var(--glass-hi);
    border-color: var(--glass-edge);
    border-radius: 14px;
}
.launcher-row, .cat-item, .menu-item, .grid-cell { border-radius: 12px; }
.launcher-row-selected, .cat-item-active, .grid-cell-selected {
    background-color: var(--glass-hi);
    border-width: 1px;
    border-color: var(--glass-edge);
}
.launcher-icon-glyph, .grid-icon-glyph, .popup-big-icon, .osd-icon { icon-color: var(--accent); }
.notif-in-center { background-color: var(--glass-hi); }
.notif-action, .round-btn, .power-btn, .chip { background-color: var(--glass-hi); }
.meter-fill { background: linear-gradient(90deg, var(--accent), var(--violet)); }
.meter-rest { background-color: var(--glass-hi); }
.badge { background-color: var(--violet); }
.lock-avatar { background: linear-gradient(135deg, var(--accent), var(--violet)); }
