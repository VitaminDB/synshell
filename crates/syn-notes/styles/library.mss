/* Библиотека заметок (карточки блокнотов, рисунков, книг) и окно с CSD. */

/* ── Окно ── */
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
    padding: 0px 6px 0px 12px;
    background-color: var(--bg-shell);
    border-bottom: 1px solid var(--border-soft);
}
.tb-brand {
    font-size: 14px;
    font-weight: 600;
    color: var(--text);
}
.tb-brand-icon {
    icon-size: 20px;
    color: var(--primary);
}
.drag-space {
    padding: 0px 12px;
}
.tb-title {
    font-size: 13px;
    color: var(--text-muted);
}
.tb-btn {
    width: 36px;
    height: 36px;
    icon-size: 20px;
    border-radius: 10px;
    color: var(--text-muted);
}
.tb-btn:hover {
    background-color: var(--surface-hover);
}
.tb-btn.active {
    color: var(--primary);
    background-color: var(--primary-soft);
}
.window-controls {
    padding: 0px 0px 0px 6px;
}

/* ── Библиотека ── */
.lib {
    background-color: var(--bg-shell);
}
.lib-bar {
    min-height: 56px;
    padding: 6px 12px;
    background-color: var(--bg-shell);
    border-bottom: 1px solid var(--border-soft);
}
.lib-bar-icon {
    icon-size: 24px;
    color: var(--text);
    margin: 0px 6px 0px 4px;
}
.lib-title {
    font-size: 20px;
    font-weight: 600;
    color: var(--text);
}
.lib-open-title {
    font-size: 17px;
    font-weight: 600;
    color: var(--text);
    padding: 0px 6px;
}
.lib-btn {
    width: 44px;
    height: 44px;
    icon-size: 24px;
    border-radius: 12px;
    color: var(--text);
}
.lib-btn:hover {
    background-color: var(--surface-hover);
}
.lib-btn.active {
    color: var(--on-primary);
    background-color: var(--text);
}
.lib-create {
    height: 44px;
    padding: 0px 16px 0px 12px;
    margin: 0px 0px 0px 6px;
    border-radius: 22px;
    background-color: var(--text);
}
.lib-create-icon {
    icon-size: 22px;
    color: var(--bg-shell);
}
.lib-create-text {
    font-size: 15px;
    text-box-edge: text;
    font-weight: 600;
    color: var(--bg-shell);
}
.lib-search {
    height: 42px;
    font-size: 16px;
}

/* Вкладки разделов — «таблетки» с чёткой рамкой (E Ink). */
.lib-tabs {
    padding: 10px 16px 4px 16px;
}
.lib-tab {
    height: 38px;
    padding: 0px 16px;
    border-radius: 19px;
    border: 1px solid var(--border-strong);
}
.lib-tab-text {
    font-size: 15px;
    text-box-edge: text;
    color: var(--text);
}
.lib-tab.active {
    background-color: var(--text);
    border: 1px solid var(--text);
}
.lib-tab.active .lib-tab-text {
    color: var(--bg-shell);
    font-weight: 600;
}

.lib-crumbs {
    padding: 8px 20px 0px 20px;
}
.lib-crumb {
    font-size: 15px;
    color: var(--text-muted);
}
.lib-crumb-sep {
    icon-size: 18px;
    color: var(--text-subtle);
}

.lib-grid {
    padding: 16px 20px 32px 20px;
}

/* Карточка: обложка 3:4 с рамкой, название, подробности. */
.lib-card {
    padding: 4px;
    border-radius: 12px;
}
.lib-card:hover {
    background-color: var(--surface-hover);
}
.lib-thumb {
    border-radius: 8px;
    border: 1px solid var(--border-strong);
    background-color: #FFFFFF;
}
.lib-thumb.folder {
    background-color: var(--surface-hover);
    border: 1px solid var(--border);
}
.lib-thumb-icon {
    font-size: 48px;
    icon-size: 48px;
    color: var(--text-subtle);
}
.lib-thumb-doc {
    padding: 10px;
}
.lib-thumb-doc-text {
    font-size: 9px;
    line-height: 1.35;
    color: #303030;
}
.lib-card-kind {
    icon-size: 16px;
    color: var(--text-muted);
}
.lib-card-title {
    font-size: 15px;
    font-weight: 600;
    color: var(--text);
}
.lib-card-sub {
    font-size: 13px;
    color: var(--text-muted);
}

/* Список. */
.lib-row {
    min-height: 60px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border-soft);
}
.lib-row:hover {
    background-color: var(--surface-hover);
}
.lib-row-icon {
    icon-size: 26px;
    color: var(--text);
}

.lib-empty {
    padding: 80px 24px;
}

/* Открытая страница. */
.lib-open-body {
    background-color: var(--bg-shell);
}

/* Диалог переименования. */
.lib-dialog {
    width: 420px;
    padding: 20px;
    border-radius: 16px;
    background-color: var(--bg-panel);
    border: 1px solid var(--border-strong);
}
.lib-dialog-title {
    font-size: 17px;
    font-weight: 600;
    color: var(--text);
}
.lib-dialog-btn {
    height: 40px;
    padding: 0px 18px;
    border-radius: 20px;
    border: 1px solid var(--border-strong);
    color: var(--text);
}
.lib-dialog-btn.primary {
    background-color: var(--text);
    color: var(--bg-shell);
    border: 1px solid var(--text);
}
.lib-create.compact {
    width: 44px;
    padding: 0px 0px 0px 11px;
}
.lib-tabs {
    height: 56px;
}
.lib-tabs-row {
    padding: 0px 16px 0px 0px;
}
