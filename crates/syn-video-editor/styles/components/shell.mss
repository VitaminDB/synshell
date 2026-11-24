/* Окно без системной рамки: «воздух» вокруг шелла для захвата краёв,
 * в развёрнутом состоянии — во весь экран без скруглений. */
.window-backdrop {
    background-color: var(--bg-window);
    padding: 10px;
    transition: padding 180ms ease;
}

.window-backdrop:window-maximized {
    padding: 0;
}

.shell {
    background-color: var(--bg-shell);
    border-radius: var(--radius-shell);
    border-width: 1px;
    border-color: var(--border);
    box-shadow: 0 6px 10px rgba(0, 0, 0, 0.35);
    transition: border-radius 180ms ease;
}

.shell:window-maximized {
    border-radius: 0;
    border-width: 0;
    box-shadow: none;
}

/* ── заголовок окна: логотип, вкладки файлов, кнопки окна ── */
.titlebar {
    height: 40px;
    padding: 0 0 0 14px;
    background-color: var(--bg-shell);
}

.logo-mark {
    width: 22px;
    height: 22px;
    justify-content: center;
    align-items: center;
}

.logo-mark-text {
    color: #FFFFFF;
    font-size: 11px;
    font-weight: 700;
}

.logo-text {
    color: var(--text);
    font-size: 13px;
    font-weight: 600;
}

/* чип с номером сборки рядом с логотипом */
.build-chip {
    height: 20px;
    padding: 0px 8px 0px 7px;
    border-radius: 10px;
    border-width: 1px;
    border-color: var(--border-soft);
    background-color: var(--primary-soft);
}

.build-chip-dot {
    width: 6px;
    height: 6px;
    border-radius: 3px;
    background-color: var(--primary);
}

.build-chip-text {
    font-family: monospace;
    font-size: 11px;
    font-weight: 600;
    color: var(--text);
}

.window-control {
    width: 46px;
    height: 40px;
    background-color: transparent;
    cursor: pointer;
    transition: background-color 120ms ease-out;
}

.window-control-icon {
    color: var(--text-muted);
    icon-size: 16px;
}

.window-control:hover { background-color: var(--surface-hover); }
.window-control.close:hover { background-color: #E81123; }
.window-control.close:hover .window-control-icon { color: #FFFFFF; }

/* ── вкладки файлов ── */
.file-tab {
    height: 30px;
    padding: 0 6px 0 12px;
    border-radius: 8px;
    background-color: transparent;
    transition: background-color 120ms ease-out;
}

.file-tab:hover { background-color: var(--surface-hover); }

.file-tab-active {
    background-color: var(--bg-panel-2);
    border-width: 1px;
    border-color: var(--border);
}

.file-tab-title {
    color: var(--text-muted);
    font-size: 12px;
}

.file-tab-active .file-tab-title { color: var(--text); }

.file-tab-dot {
    width: 7px;
    height: 7px;
    border-radius: 4px;
    background-color: var(--modified);
}

.file-tab-close {
    icon-size: 14px;
    padding: 3px;
    border-radius: 5px;
}
