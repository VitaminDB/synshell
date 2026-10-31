/* Читалка книг (notes/reader): страница, подвал, меню по тапу, панели.
   E Ink: без теней и прозрачных слоёв поверх текста — чёткие рамки. */

.reader {
    background-color: var(--bg-shell);
}
.reader-area {
    background-color: var(--bg-shell);
}
.reader-paper {
    background-color: var(--bg-shell);
}
/* Текст страницы: color — буквы, accent-color — ссылки и выделение. */
.reader-page {
    color: var(--text);
    accent-color: var(--primary);
}
.reader-msg {
    font-size: 15px;
    color: var(--text-muted);
    padding: 16px;
}
.reader-footer {
    height: 28px;
    padding: 0px 16px;
    background-color: var(--bg-shell);
}
.reader-footer-text {
    font-size: 12px;
    color: var(--text-muted);
}

/* Кнопки */
.reader-btn {
    width: 44px;
    height: 44px;
    icon-size: 24px;
    border-radius: 10px;
    color: var(--text);
}
.reader-btn:hover {
    background-color: var(--surface-hover);
}
.reader-btn.active {
    color: var(--primary);
    background-color: var(--primary-soft);
}
.reader-action {
    min-width: 64px;
    padding: 6px 4px;
    border-radius: 10px;
}
.reader-action:hover {
    background-color: var(--surface-hover);
}
.reader-action-icon {
    icon-size: 26px;
    color: var(--text);
}
.reader-action-label {
    font-size: 12px;
    color: var(--text-muted);
}

/* Меню и панели */
.reader-sheet {
    background-color: var(--bg-shell);
    padding: 10px 14px;
}
.reader-sheet-top {
    border-bottom: 1px solid var(--border-strong);
}
.reader-sheet-bottom {
    border-top: 1px solid var(--border-strong);
    max-height: 560px;
}
.reader-menu-title {
    font-size: 16px;
    font-weight: 600;
    color: var(--text);
}
.reader-menu-sub {
    font-size: 13px;
    color: var(--text-muted);
}
.reader-side {
    width: 380px;
    padding: 12px;
    background-color: var(--bg-shell);
    border-right: 1px solid var(--border-strong);
}
.reader-scrim {
    background-color: rgba(0, 0, 0, 0.08);
}
.reader-sheet-title {
    font-size: 17px;
    font-weight: 600;
    color: var(--text);
}
.reader-tab {
    padding: 6px 12px;
    border-radius: 8px;
    border: 1px solid var(--border);
    font-size: 13px;
    color: var(--text-muted);
}
.reader-tab.active {
    border: 1px solid var(--text);
    color: var(--text);
    font-weight: 600;
}
.reader-row {
    padding: 10px 12px;
    border-bottom: 1px solid var(--border-soft);
}
.reader-row:hover {
    background-color: var(--surface-hover);
}
.reader-row.current {
    background-color: var(--surface-selected);
}
.reader-row-title {
    font-size: 14px;
    color: var(--text);
}
.reader-row-sub {
    font-size: 12px;
    color: var(--text-muted);
}
.reader-setting {
    min-height: 44px;
    font-size: 14px;
    color: var(--text);
}
.reader-value {
    min-width: 48px;
    font-size: 14px;
    text-align: center;
}
.reader-chip {
    padding: 6px 12px;
    border-radius: 16px;
    border: 1px solid var(--border);
    font-size: 13px;
    color: var(--text);
}
.reader-chip.active {
    border: 2px solid var(--text);
    font-weight: 600;
}
.reader-selbar {
    padding: 6px 10px;
    margin: 0px 0px 40px 0px;
    border-radius: 14px;
    background-color: var(--bg-shell);
    border: 1px solid var(--border-strong);
}
.reader-dialog {
    width: 420px;
    padding: 18px;
    border-radius: 14px;
    background-color: var(--bg-shell);
    border: 1px solid var(--border-strong);
}
.reader-quote {
    font-size: 14px;
    color: var(--text-muted);
}

/* Обложка в библиотеке */
.reader-cover {
    background-color: var(--bg-shell);
}
.reader-cover-typeset {
    padding: 14px 10px;
    border: 1px solid var(--border-strong);
}
.reader-cover-title {
    font-size: 15px;
    font-weight: 600;
    color: var(--text);
    text-align: center;
}
.reader-cover-author {
    font-size: 12px;
    color: var(--text-muted);
    text-align: center;
}
.reader-cover-format {
    font-size: 11px;
    color: var(--text-subtle);
}
