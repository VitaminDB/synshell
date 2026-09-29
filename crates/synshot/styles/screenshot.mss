/* Снимок экрана (synshot): панель действий и подсказка
   поверх застывшего экрана. Цвета — палитра рабочего стола. */

.shot-overlay { accent-color: var(--accent); }

.shot-bar {
    background: var(--menu-bg);
    border: 1px solid var(--border);
    border-radius: 14px;
    padding: 5px 6px;
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.45);
    color: var(--fg);
}

Button.shot-btn {
    height: 34px;
    padding: 0px 12px;
    border-radius: 10px;
    border-width: 0px;
    background: transparent;
    color: var(--fg);
    font-size: 13px;
    icon-size: 18px;
    transition: background 100ms ease;
    &:hover { background: var(--hover); }
    &:pressed { background: var(--pressed); }
}
Button.shot-btn.primary {
    background: var(--accent);
    color: var(--accent-fg);
    font-weight: 600;
    &:hover { background: var(--accent); opacity: 0.9; }
}

ToolButton.shot-close {
    width: 34px;
    height: 34px;
    border-radius: 10px;
    background: transparent;
    color: var(--muted);
    icon-size: 20px;
    &:hover { background: var(--hover); color: var(--danger); }
}

.shot-sep { width: 1px; height: 22px; margin: 0px 5px; background: var(--border); }

.shot-hint-icon { color: var(--accent); font-size: 22px; icon-size: 22px; margin: 0px 8px 0px 6px; }
.shot-hint-text { color: var(--fg); font-size: 13px; font-weight: 600; }
.shot-hint-keys { color: var(--muted); font-size: 11.5px; }
