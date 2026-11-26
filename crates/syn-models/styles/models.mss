/* «Модели»: окно (CSD), заголовок с разделами. Цвета — из палитры оформления. */

.window-frame {
    background-color: var(--bg-shell);
    border-radius: var(--radius-shell);
    border: 1px solid var(--border);
}
.window-frame.flat {
    border-radius: 0px;
    border: 0px solid var(--border);
}
.titlebar {
    height: 44px;
    padding: 0px 6px 0px 14px;
    background-color: var(--bg-rail);
}
.tb-brand {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
}
.tb-brand-icon {
    icon-size: 18px;
    color: var(--primary);
}
.drag-space {
    padding: 0px 12px;
}
.tb-btn {
    width: 34px;
    height: 34px;
    icon-size: 18px;
    border-radius: 9px;
    color: var(--text-muted);
}
.tb-btn:hover {
    background-color: var(--surface-hover);
}
.window-controls {
    padding: 0px 0px 0px 6px;
}

/* Разделы в заголовке */
.tb-tab {
    height: 30px;
    padding: 0px 12px;
    border-radius: 8px;
    color: var(--text-muted);
    background-color: transparent;
    font-size: 13px;
}
.tb-tab:hover {
    background-color: var(--surface-hover);
}
.tb-tab.selected {
    background-color: var(--primary-soft);
    color: var(--text);
}
.tb-tab-icon {
    icon-size: 16px;
    color: inherit;
}

.page-body {
    padding: 0px 6px 6px 6px;
}

/* Настройки (как в synthos) */
.models-settings {
    padding: 24px 32px 48px 32px;
}
.settings-page-title {
    font-size: 24px;
    font-weight: 700;
    color: var(--text);
}

.settings-page-subtitle {
    font-size: 14px;
    color: var(--text-muted);
}

.settings-section-title {
    font-size: 13px;
    font-weight: 700;
    color: var(--text-subtle);
    letter-spacing: 0.6px;
}

.settings-card {
    background-color: var(--bg-panel);
    border-radius: var(--radius-panel);
    border-width: 1px;
    border-color: var(--border);
    /* Clip по скруглённым углам: без него border-bottom разделителей
     * строк (settings-row / models-active-row) торчит за контур карты. */
    overflow: hidden;
    transition: border-color var(--duration-med) var(--ease-standard),
                background-color var(--duration-med) var(--ease-standard);
}

.tb-gap {
    width: 16px;
}
