/* Catppuccin: «таблетки», лавандовый акцент, розовые бейджи. */

.panel { border-color: var(--surface-alt); }

.applet, .task, .tray-item { border-radius: 12px; }
.ws { border-radius: 12px; min-width: 22px; padding: 4px 10px; }
.ws-active { background-color: var(--accent); }
.ws-active:hover { background-color: var(--accent); }
.ws-active .ws-label { color: var(--accent-fg); }
.ws-active .ws-dot { background-color: var(--accent-fg); }

.task-active {
    background-color: var(--lavender-soft);
    border-color: var(--accent);
}

.badge { background-color: var(--pink); border-radius: 9px; }
.badge-text { color: var(--surface); }

.popup-card, .notif, .osd, .switcher {
    border-radius: 18px;
    box-shadow: 0 14px 40px var(--shadow);
}

.search-box { border-radius: 14px; }
.cat-item, .launcher-row, .menu-item, .grid-cell { border-radius: 12px; }
.launcher-row-selected, .cat-item-active, .grid-cell-selected { background-color: var(--lavender-soft); }
.launcher-icon-glyph, .grid-icon-glyph, .popup-big-icon, .osd-icon { icon-color: var(--pink); }

.chip { border-radius: 16px; }
.chip-on { background-color: var(--accent); }
.chip-on .chip-label { color: var(--accent-fg); }
.meter-fill { background-color: var(--accent); }
.round-btn { border-radius: 17px; }

.lock-avatar { background-color: var(--accent); }
.lock-field { border-radius: 22px; }
