.toolbar {
    height: 44px;
    padding: 0 12px;
    background-color: var(--bg-shell);
}

.toolbar-sep {
    width: 1px;
    height: 20px;
    background-color: var(--border);
}

.toolbar-label {
    color: var(--text-subtle);
    font-size: 12px;
}

.search-field {
    width: 260px;
    height: 30px;
}

.goto-field {
    width: 130px;
}

/* ключевые действия — цветом */
.tb-run {
    color: var(--success);
}

.tb-stop {
    color: var(--error);
}

.tb-ai {
    color: var(--primary-hover);
}

/* вкладки страниц: анализ / скрипты / настройки */
.page-tab {
    padding: 5px 12px;
    border-radius: 8px;
    transition: background-color 120ms;
}

.page-tab:hover { background-color: var(--surface-hover); }
.page-tab.on { background-color: var(--primary-soft); }

.page-tab-icon {
    icon-size: 16px;
    color: var(--text-muted);
}

.page-tab.on .page-tab-icon { color: var(--primary-hover); }

.page-tab-text {
    font-size: 13px;
    color: var(--text-muted);
}

.page-tab.on .page-tab-text { color: var(--text); }

/* «▾» рядом с «Собрать»: меню сборки (тулчейн, конфигурация, запуск) */
.tb-dropdown {
    width: 18px;
    height: 28px;
    border-radius: 6px;
    padding: 6px 1px 6px 1px;
}
.tb-dropdown:hover {
    background-color: var(--hover);
}
.tb-dropdown-icon {
    font-size: 16px;
    color: var(--text-muted);
}
