/* Rosé Pine: мягкие формы, золото и ирис в деталях. */

.panel { border-color: var(--surface-alt); }
.ws, .task, .applet, .tray-item { border-radius: 10px; }

.ws-active { background-color: var(--rose-soft); border-width: 1px; border-color: var(--accent); }
.ws-active .ws-label { color: var(--accent); }
.ws-active .ws-dot { background-color: var(--gold); }

.task-active { background-color: var(--rose-soft); border-color: var(--rose-soft); }

.clock-time { color: var(--accent); }
.clock-date { color: var(--iris); }
.applet-launcher .icon { icon-color: var(--iris); }

.popup-card, .notif, .osd, .switcher {
    border-radius: 20px;
    border-color: var(--surface-alt);
    box-shadow: 0 18px 48px var(--shadow);
}
.popup-title, .notif-summary { color: var(--accent); }

.search-box { border-radius: 14px; }
.launcher-row, .cat-item, .menu-item, .grid-cell { border-radius: 12px; }
.launcher-row-selected, .cat-item-active, .grid-cell-selected { background-color: var(--rose-soft); }
.launcher-icon-glyph, .grid-icon-glyph, .popup-big-icon, .osd-icon { icon-color: var(--iris); }
.meter-fill { background: linear-gradient(90deg, var(--iris), var(--accent)); }
.badge { background-color: var(--gold); }
.lock-avatar { background: linear-gradient(135deg, var(--iris), var(--accent)); }
