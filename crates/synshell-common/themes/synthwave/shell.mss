/* Синтвейв: неон на всём, что активно. */

.panel {
    background: linear-gradient(90deg, #1a1028e6, #2a1446e6, #1a1028e6);
    border-color: var(--neon-pink);
    box-shadow: 0 0 16px var(--pink-soft);
}

.applet-label, .ws-label, .task-title { letter-spacing: 1px; }

.ws-active {
    background-color: var(--cyan-soft);
    border-width: 1px;
    border-color: var(--neon-cyan);
    glow: 0 0 12px var(--cyan-glow);
}
.ws-active .ws-label { color: var(--neon-cyan); }
.ws-active .ws-dot { background-color: var(--neon-cyan); }

.task-active {
    background: linear-gradient(90deg, var(--pink-soft), var(--cyan-soft));
    border-color: var(--neon-pink);
}
.task:hover { background-color: var(--pink-soft); }

.clock-time {
    color: var(--neon-cyan);
    text-shadow: 0 0 8px var(--neon-cyan);
    letter-spacing: 2px;
}
.clock-date { color: var(--neon-pink); text-transform: uppercase; }
.keyboard-label { color: var(--neon-yellow); }
.applet-launcher .icon { icon-color: var(--neon-pink); }

.popup-card, .notif, .osd, .switcher {
    border-color: var(--neon-pink);
    box-shadow: 0 0 22px var(--pink-glow), 0 0 2px var(--neon-pink);
}
.popup-title, .notif-summary {
    color: var(--neon-cyan);
    text-transform: uppercase;
    letter-spacing: 2px;
}
.notif-critical { border-color: var(--danger); }

.search-box { border-color: var(--neon-cyan); box-shadow: 0 0 10px var(--cyan-soft); }
.search-field { caret-color: var(--neon-cyan); }
.launcher-row-selected, .cat-item-active, .grid-cell-selected {
    background: linear-gradient(90deg, #ff4fb855, #7b2cbf33);
    border-width: 1px;
    border-color: var(--neon-pink);
}
.launcher-name { letter-spacing: 1px; }
.launcher-user { color: var(--neon-yellow); text-transform: uppercase; letter-spacing: 2px; }
.launcher-icon-glyph, .grid-icon-glyph, .popup-big-icon, .osd-icon { icon-color: var(--neon-cyan); }

.meter-fill { background: linear-gradient(90deg, var(--neon-pink), var(--neon-cyan)); glow: 0 0 8px var(--pink-glow); }
.chip-on { background-color: var(--pink-soft); border-width: 1px; border-color: var(--neon-pink); }
.badge { background-color: var(--neon-yellow); }
.badge-text { color: #1a1028; }
.switch-card-selected { border-color: var(--neon-cyan); glow: 0 0 16px var(--cyan-glow); }

.cal { accent-color: var(--neon-pink); }
.lock-time { color: var(--neon-cyan); text-shadow: 0 0 24px var(--neon-cyan); }
.lock-avatar { background: linear-gradient(135deg, var(--neon-pink), #7b2cbf); glow: 0 0 24px var(--pink-glow); }
.lock-field { border-color: var(--neon-pink); }
