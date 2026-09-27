/* Gruvbox: ретро-плашки, жирные подписи, ржавый акцент. */

.panel { border-width: 2px; border-color: var(--border); }

.applet, .task, .ws, .tray-item, .menu-item, .cat-item, .launcher-row, .grid-cell { border-radius: 3px; }

.ws-label { font-weight: bold; }
.ws-active { background-color: var(--accent); }
.ws-active:hover { background-color: var(--accent); }
.ws-active .ws-label { color: var(--accent-fg); }
.ws-active .ws-dot { background-color: var(--accent-fg); }

.task-active { background-color: var(--rust-soft); border-color: var(--accent); }
.task-title { font-weight: bold; }

.clock-time { color: var(--yellow); font-size: 15px; }
.applet-launcher .icon { icon-color: var(--yellow); }

.popup-card, .notif, .osd, .switcher {
    border-width: 2px;
    border-radius: 4px;
    box-shadow: 6px 6px 0 var(--shadow);
}
.popup-title, .notif-summary, .launcher-user { color: var(--yellow); }

.search-box { border-width: 2px; border-radius: 3px; }
.launcher-row-selected, .cat-item-active, .grid-cell-selected { background-color: var(--rust-soft); }
.launcher-icon-glyph, .grid-icon-glyph, .osd-icon, .popup-big-icon { icon-color: var(--aqua); }
.meter-fill, .meter-rest { border-radius: 0px; }
.chip { border-radius: 3px; }
.badge { border-radius: 2px; background-color: var(--danger); }
