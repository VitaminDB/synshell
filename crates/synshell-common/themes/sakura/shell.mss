/* Сакура: лепестки — мягкие «таблетки» и розовая тень. */

.panel { border-color: var(--border); }
.ws, .task, .applet, .tray-item { border-radius: 14px; }

.ws-active { background-color: var(--accent); }
.ws-active:hover { background-color: var(--accent); }
.ws-active .ws-label { color: var(--accent-fg); }
.ws-active .ws-dot { background-color: var(--accent-fg); }

.task-active { background-color: var(--petal-soft); border-color: var(--petal-soft); }

.clock-time { color: var(--petal); }
.applet-launcher .icon { icon-color: var(--petal); }

.popup-card, .notif, .osd, .switcher {
    border-radius: 22px;
    box-shadow: 0 16px 44px var(--shadow);
}
.popup-title, .notif-summary { color: var(--petal); }

.search-box { border-radius: 18px; }
.launcher-row, .cat-item, .menu-item, .grid-cell { border-radius: 14px; }
.launcher-row-selected, .cat-item-active, .grid-cell-selected { background-color: var(--petal-soft); }
.launcher-icon-glyph, .grid-icon-glyph, .popup-big-icon, .osd-icon { icon-color: var(--petal); }
.chip { border-radius: 16px; }
.meter-fill { background: linear-gradient(90deg, var(--petal), var(--leaf)); }
.badge { background-color: var(--leaf); }
.lock-field { border-radius: 22px; }
