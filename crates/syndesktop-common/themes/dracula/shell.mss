/* Dracula: фиолетовый с розовыми вспышками. */

.panel { border-color: var(--border); }

.ws-active { background-color: var(--purple-soft); border-width: 1px; border-color: var(--accent); }
.ws-active .ws-label { color: var(--accent); }
.ws-active .ws-dot { background-color: var(--pink); }

.task-active { background-color: var(--purple-soft); border-color: var(--accent); }
.task:hover { background-color: var(--pink-soft); }

.clock-time { color: var(--pink); }
.clock-date { color: var(--cyan); }
.applet-launcher .icon { icon-color: var(--pink); }

.popup-card, .notif, .osd, .switcher {
    border-color: var(--accent);
    box-shadow: 0 0 18px var(--purple-soft), 0 14px 40px var(--shadow);
}
.popup-title, .notif-summary { color: var(--pink); }

.search-box { border-color: var(--accent); }
.search-field { caret-color: var(--pink); }
.launcher-row-selected, .cat-item-active, .grid-cell-selected { background-color: var(--purple-soft); }
.launcher-icon-glyph, .grid-icon-glyph, .popup-big-icon, .osd-icon { icon-color: var(--cyan); }
.meter-fill { background: linear-gradient(90deg, var(--accent), var(--pink)); }
.badge { background-color: var(--pink); }
.lock-avatar { background: linear-gradient(135deg, var(--accent), var(--pink)); }
