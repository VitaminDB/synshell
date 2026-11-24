/* Стандартные виджеты syngui в теме приложения: без этих правил они рисуют
 * свои светлые дефолты (белые таблицы, выпадающие списки, редактор). */

TextField {
    background-color: var(--bg-field);
    color: var(--text);
    border-radius: var(--radius-field);
    border-width: 1px;
    border-color: var(--border);
    accent-color: var(--primary);
    font-size: 13px;
    /* компактно: высота поля считается по шрифту и отступам (~30 px) */
    padding: 6px 10px;
}

MultilineTextEdit {
    background-color: var(--bg-field);
    color: var(--text);
    border-radius: var(--radius-field);
    border-width: 1px;
    border-color: var(--border);
    accent-color: var(--primary);
    selection-color: #4F8CFF55;
    caret-color: var(--primary);
    font-size: 13px;
    transition: border-color 140ms ease-out;
}

MultilineTextEdit:focus {
    border-color: var(--primary);
}

Dropdown {
    background-color: var(--bg-field);
    color: var(--text);
    border-radius: 8px;
    border-width: 1px;
    border-color: var(--border);
    padding: 5px 10px;
    font-size: 12px;
    accent-color: var(--primary);
    /* длинные пункты меню переносятся на строки (во всём приложении) */
    --popup-wrap: true;
    --popup-background: var(--bg-panel-2);
    --popup-color: var(--text);
    --popup-border: var(--border);
    --popup-accent: var(--primary);
    --popup-hover-background: var(--surface-hover);
    --popup-hover-color: var(--text);
}

Dropdown:hover {
    border-color: var(--primary);
}

TableView {
    --popup-background: var(--bg-panel-2);
    --popup-color: var(--text);
    --popup-border: var(--border);
    --popup-hover-background: var(--surface-hover);
}

PopupMenu {
    --popup-background: var(--bg-panel-2);
    --popup-color: var(--text);
    --popup-border: var(--border);
    --popup-accent: var(--primary);
    --popup-hover-background: var(--surface-hover);
    --popup-hover-color: var(--text);
}

ToolButton {
    color: var(--text-muted);
    accent-color: var(--primary);
    border-radius: 7px;
}

TableView {
    background-color: transparent;
    color: var(--text);
    border-color: var(--border-soft);
    header-bg: var(--bg-panel);
    header-color: var(--text-subtle);
    row-hover-bg: var(--surface-hover);
    row-selected-bg: var(--primary-soft);
    row-striped-bg: transparent;
    scrollbar-width: 8px;
    scrollbar-radius: 4px;
    scrollbar-color: #FFFFFF2A;
}

/* Редактор кода: палитра в духе One Dark, приглушённая под тему. */
CodeEditor {
    background-color: var(--bg-hex);
    color: #D4D9E3;
    font-family: monospace;
    font-size: 13px;
    editor-bg: var(--bg-hex);
    editor-fg: #D4D9E3;
    editor-gutter-bg: var(--bg-hex);
    editor-gutter-fg: #4A5468;
    editor-cursor: var(--primary);
    editor-selection: #4F8CFF40;
    editor-current-line: #FFFFFF08;
    editor-bracket-match: #4F8CFF55;
    editor-whitespace: #FFFFFF14;
    token-keyword: #C678DD;
    token-keyword-control: #C678DD;
    token-type: #E5C07B;
    token-type-builtin: #56B6C2;
    token-function: #61AFEF;
    token-function-macro: #61AFEF;
    token-constant: #D19A66;
    token-string: #98C379;
    token-string-special: #98C379;
    token-number: #D19A66;
    token-comment: #5C6370;
    token-operator: #ABB2BF;
    token-punctuation: #ABB2BF;
    token-variable: #E06C75;
    token-property: #E06C75;
    token-attribute: #D19A66;
    token-namespace: #E5C07B;
    token-tag: #E06C75;
    scrollbar-width: 8px;
    scrollbar-radius: 4px;
    scrollbar-color: #FFFFFF2A;
}

/* Сегменты, счётчики, выбор времени — та же компактная высота, что у полей */
SegmentedButton,
SpinBox {
    font-size: 12px;
    padding-top: 6px;
    padding-bottom: 6px;
}

/* флажок: без правила невыбранный рисуется тёмной рамкой на тёмном фоне */
Checkbox {
    background-color: var(--bg-field);
    border-color: var(--text-subtle);
    color: var(--text);
    accent-color: var(--primary);
}
