/* Окно syn-notes: панель проектов слева. */
.notes-app-rail {
    width: 56px;
    padding: 10px 0px;
    background-color: var(--bg-rail);
    border-right: 1px solid var(--border-soft);
}
.notes-app-tile {
    width: 40px;
    height: 40px;
    border-radius: 12px;
    background-color: var(--surface-hover);
    transition: background-color var(--duration-fast) var(--ease-standard);
}
.notes-app-tile:hover {
    background-color: var(--surface-selected);
}
.notes-app-tile.selected {
    background-color: var(--primary-soft);
}
.notes-app-tile-letter {
    font-size: 17px;
    font-weight: 600;
    color: var(--text-muted);
}
.notes-app-tile.selected .notes-app-tile-letter {
    color: var(--primary);
}
.notes-app-add {
    width: 40px;
    height: 40px;
    icon-size: 22px;
    border-radius: 12px;
    color: var(--text-muted);
}
.notes-app-add:hover {
    background-color: var(--surface-hover);
}

/* Окно с рамкой композитора: без своего «парящего» каркаса synthos. */
.window-backdrop {
    background-color: var(--bg-shell);
}
.shell {
    background-color: var(--bg-shell);
}
.notes-app-tile-main {
    icon-size: 12px;
    color: var(--warning);
    margin: 2px 0px 0px 26px;
}
