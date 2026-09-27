/* Необрутализм: 2px рамки, тени со смещением без размытия, капслок. */

.panel {
    border-width: 2px;
    border-color: var(--border);
    border-radius: 0px;
}
.panel-floating { box-shadow: 4px 4px 0 var(--hard); }

.applet, .task, .ws, .tray-item, .menu-item, .cat-item, .launcher-row, .grid-cell, .chip, .round-btn, .power-btn {
    border-radius: 0px;
}

.ws { border-width: 2px; border-color: #00000000; }
.ws-label { font-weight: bold; }
.ws-active { background-color: var(--accent); border-color: var(--border); }
.ws-active:hover { background-color: var(--accent); }
.ws-active .ws-label { color: var(--accent-fg); }
.ws-active .ws-dot { background-color: var(--accent-fg); }

.task { border-width: 2px; }
.task-active {
    background-color: var(--pop);
    border-color: var(--border);
    box-shadow: 3px 3px 0 var(--hard);
}
.task-active .task-title { color: #111111; font-weight: bold; }
.task-urgent { background-color: var(--warning); border-color: var(--border); }

.clock-time { font-size: 15px; font-weight: bold; }
.clock-date { color: var(--fg); text-transform: uppercase; font-weight: bold; }
.keyboard-label { text-transform: uppercase; }
.applet-launcher .icon { icon-color: var(--fg); }

.popup-card, .notif, .osd, .switcher {
    border-width: 2px;
    border-color: var(--border);
    border-radius: 0px;
    box-shadow: 6px 6px 0 var(--hard);
}
.popup-title, .notif-summary, .launcher-user { text-transform: uppercase; letter-spacing: 1px; }

.search-box { border-width: 2px; border-radius: 0px; background-color: var(--surface); }
.launcher-row-selected, .cat-item-active, .grid-cell-selected {
    background-color: var(--pop2);
    border-width: 2px;
    border-color: var(--border);
}
.launcher-row-selected .launcher-name, .launcher-row-selected .launcher-sub,
.cat-item-active .menu-label, .grid-cell-selected .grid-name { color: #111111; }
.cat-item-active .menu-icon { icon-color: #111111; }
.launcher-icon-glyph, .grid-icon-glyph, .popup-big-icon, .osd-icon { icon-color: var(--fg); }

.notif-action { border-radius: 0px; border-width: 2px; border-color: var(--border); }
.meter-fill, .meter-rest { border-radius: 0px; }
.meter-fill { background-color: var(--accent); }
.badge { border-radius: 0px; background-color: var(--danger); }
.switch-card { border-radius: 0px; }
.switch-card-selected { background-color: var(--pop2); border-color: var(--border); box-shadow: 4px 4px 0 var(--hard); }
.notif-image { border-radius: 0px; }
.lock-avatar { border-radius: 0px; border-width: 3px; border-color: var(--border); box-shadow: 6px 6px 0 var(--hard); }
.lock-field { border-radius: 0px; border-width: 2px; }
