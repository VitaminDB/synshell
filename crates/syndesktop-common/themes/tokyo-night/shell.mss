/* Tokyo Night: неоновые подсветки активного. */

.panel { border-color: var(--border); }

.ws-active {
    background-color: var(--neon-soft);
    border-width: 1px;
    border-color: var(--neon);
    glow: 0 0 10px var(--neon);
}
.ws-active .ws-label { color: var(--accent); }
.ws-active .ws-dot { background-color: var(--magenta); }

.task-active {
    background-color: var(--neon-soft);
    border-color: var(--neon);
}
.task-urgent { border-color: var(--magenta); }

.clock-time { color: var(--accent); letter-spacing: 1px; }
.clock-date { color: var(--magenta); }
.applet-launcher .icon { icon-color: var(--magenta); }

.popup-card, .notif, .osd, .switcher {
    border-color: var(--neon);
    box-shadow: 0 0 1px var(--neon), 0 16px 40px var(--shadow);
}

.search-box { border-color: var(--neon); }
.launcher-row-selected, .cat-item-active, .grid-cell-selected {
    background-color: var(--neon-soft);
    border-width: 1px;
    border-color: var(--neon);
}
.launcher-icon-glyph, .grid-icon-glyph, .osd-icon { icon-color: var(--cyan); }
.meter-fill { background: linear-gradient(90deg, var(--accent), var(--magenta)); }
.switch-card-selected { glow: 0 0 14px var(--neon); }
.badge { background-color: var(--magenta); }
