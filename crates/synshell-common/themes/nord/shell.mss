/* Nord: иней на панели, холодная подсветка активного. */

.panel {
    border-color: var(--frost-line);
    box-shadow: 0 -1px 0 var(--frost);
}

.task-active {
    background-color: var(--frost);
    border-color: var(--frost-line);
}
.task-active:hover { background-color: var(--accent-soft); }

.ws-active { background-color: var(--frost); border-width: 1px; border-color: var(--frost-line); }

.popup-card, .notif, .osd, .switcher {
    border-color: var(--frost-line);
    box-shadow: 0 12px 36px var(--shadow);
}

.launcher-row-selected, .cat-item-active, .grid-cell-selected {
    background-color: var(--frost);
    border-width: 1px;
    border-color: var(--frost-line);
}

.search-box { border-color: var(--frost-line); }
.clock-time { letter-spacing: 1px; }
