/* Глобальные правила по типам виджетов — чтобы ничто не выпадало из тёмной темы. */
Text {
    color: var(--text);
    font-size: 13px;
}

Icon {
    color: var(--text-muted);
    icon-size: 18px;
}

TextField {
    width: 220px;
    background-color: var(--bg-field);
    color: var(--text);
    border-radius: var(--radius-field);
    border-width: 1px;
    border-color: var(--border);
    accent-color: var(--primary);
    selection-color: #4F8CFF55;
    caret-color: var(--primary);
    padding: 7px 10px;
    font-size: 13px;
    transition: border-color 140ms ease-out;
}

TextField:focus {
    border-color: var(--primary);
}

ToolButton {
    icon-size: 18px;
    icon-color: var(--text-muted);
    icon-color-hover: var(--text);
    icon-color-disabled: var(--text-subtle);
    background-color: transparent;
    border-radius: 8px;
    padding: 6px;
    transition: background-color 120ms ease-out;
}

ToolButton:hover {
    background-color: var(--surface-hover);
}

Button {
    background-color: var(--bg-field);
    color: var(--text);
    border-radius: var(--radius-field);
    padding: 7px 14px;
    font-size: 13px;
    icon-color: var(--text-muted);
    transition: background-color 120ms ease-out;
}

Button:hover {
    background-color: var(--surface-active);
}

.btn-primary {
    background-color: var(--primary);
    color: var(--on-primary);
    icon-color: var(--on-primary);
}

.btn-primary:hover {
    background-color: var(--primary-hover);
}

/* недоступная главная кнопка — как прочие недоступные, а не бледно-синяя (выглядела нажимаемой) */
.btn-primary:disabled {
    background-color: var(--bg-field);
    color: var(--text-muted);
    icon-color: var(--text-muted);
}

SegmentedButton {
    background-color: var(--bg-field);
    border-color: var(--border);
    color: var(--text-muted);
    accent-color: var(--primary);
    border-radius: var(--radius-field);
    font-size: 12px;
}

Chip {
    background-color: var(--bg-field);
    border-color: var(--border);
    color: var(--text-muted);
    accent-color: var(--primary);
    font-size: 12px;
}

Tooltip {
    background-color: #0B0D11;
    color: var(--text);
    font-size: 12px;
    border-radius: 6px;
}

ScrollView {
    scrollbar-width: 6px;
    scrollbar-color: #FFFFFF22;
    scrollbar-thumb-hover-color: #FFFFFF44;
    scrollbar-track-color: transparent;
    scrollbar-radius: 3px;
}

Notification {
    background-color: var(--bg-panel-2);
    color: var(--text);
    border-color: var(--border);
}
