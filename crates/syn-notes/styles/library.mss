/* Заметки: окно (CSD), библиотека, открытая страница.
 * Цвета — из палитры оформления (variables), светлая и тёмная темы;
 * E Ink — поправки в eink.mss. */

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
    height: 40px;
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
.tb-title {
    font-size: 13px;
    color: var(--text-muted);
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

/* ── Каркас библиотеки ── */
.lib {
    background-color: var(--bg-shell);
}
.lib-main {
    background-color: var(--bg-shell);
}
.lib-content {
    padding: 4px 32px 48px 32px;
}

/* Боковая панель. */
.lib-side {
    width: 260px;
    background-color: var(--bg-rail);
    border-right: 1px solid var(--border-soft);
}
.lib-side-nav {
    padding: 4px 12px 24px 12px;
}
.lib-proj {
    padding: 10px 10px;
    border-radius: 14px;
    background-color: var(--bg-panel);
    border: 1px solid var(--border-soft);
}
.lib-proj:hover {
    background-color: var(--surface-hover);
}
.lib-proj-badge {
    width: 36px;
    height: 36px;
    border-radius: 10px;
    background-color: var(--primary);
}
.lib-proj-letter {
    font-size: 16px;
    font-weight: 700;
    color: var(--on-primary);
    text-box-edge: text;
}
.lib-proj-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--text);
}
.lib-proj-sub {
    font-size: 12px;
    color: var(--text-muted);
}
.lib-proj-chevron {
    icon-size: 20px;
    color: var(--text-muted);
}

.lib-nav {
    height: 40px;
    padding: 0px 12px;
    border-radius: 10px;
}
.lib-nav:hover {
    background-color: var(--surface-hover);
}
.lib-nav.active {
    background-color: var(--primary-soft);
}
.lib-nav.nested {
    height: 36px;
}
.lib-nav-icon {
    icon-size: 20px;
    color: var(--text-muted);
}
.lib-nav.active .lib-nav-icon {
    color: var(--primary);
}
.lib-nav-text {
    font-size: 14px;
    color: var(--text);
    text-box-edge: text;
}
.lib-nav.active .lib-nav-text {
    font-weight: 600;
    color: var(--primary);
}
.lib-nav-count {
    font-size: 12px;
    color: var(--text-subtle);
    text-box-edge: text;
}
.lib-side-section {
    padding: 18px 4px 6px 12px;
}
.lib-side-caption {
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.8px;
    color: var(--text-subtle);
}
.lib-side-add {
    width: 26px;
    height: 26px;
    icon-size: 18px;
    border-radius: 8px;
    color: var(--text-muted);
}
.lib-side-add:hover {
    background-color: var(--surface-hover);
}
.lib-side-hint {
    font-size: 12px;
    color: var(--text-subtle);
    padding: 2px 12px;
}

/* Нижняя навигация (узкое окно). */
.lib-bnav {
    height: 72px;
    padding: 6px 4px 8px 4px;
    background-color: var(--bg-rail);
    border-top: 1px solid var(--border-soft);
}
.lib-bnav-pill {
    width: 56px;
    height: 30px;
    border-radius: 15px;
}
.lib-bnav-pill.active {
    background-color: var(--primary-soft);
}
.lib-bnav-icon {
    icon-size: 22px;
    color: var(--text-muted);
}
.lib-bnav-pill.active .lib-bnav-icon {
    color: var(--primary);
}
.lib-bnav-text {
    font-size: 11px;
    color: var(--text-muted);
}
.lib-bnav-text.active {
    font-weight: 600;
    color: var(--text);
}

/* ── Шапка раздела ── */
.lib-header {
    padding: 22px 32px 14px 32px;
}
.lib-header.narrow {
    padding: 14px 12px 10px 16px;
}
.lib-h1 {
    font-size: 28px;
    font-weight: 700;
    color: var(--text);
}
.lib-h1.narrow {
    font-size: 24px;
}
.lib-h1-sub {
    font-size: 13px;
    color: var(--text-muted);
}
.lib-search-box {
    width: 280px;
}
.lib-search {
    height: 40px;
    font-size: 14px;
    border-radius: 20px;
}
.lib-back {
    width: 40px;
    height: 40px;
    icon-size: 22px;
    border-radius: 20px;
    color: var(--text);
}
.lib-back:hover {
    background-color: var(--surface-hover);
}
.lib-icon-btn {
    width: 40px;
    height: 40px;
    icon-size: 22px;
    border-radius: 20px;
    color: var(--text-muted);
}
.lib-icon-btn:hover {
    background-color: var(--surface-hover);
    color: var(--text);
}
.lib-icon-btn.small {
    width: 34px;
    height: 34px;
    icon-size: 20px;
}
.lib-create {
    height: 40px;
    padding: 0px 18px 0px 14px;
    margin: 0px 0px 0px 4px;
    border-radius: 20px;
    background-color: var(--primary);
}
.lib-create:hover {
    background-color: var(--primary-hover);
}
.lib-create.compact {
    width: 40px;
    padding: 0px 0px 0px 9px;
}
.lib-create-icon {
    icon-size: 22px;
    color: var(--on-primary);
}
.lib-create-text {
    font-size: 14px;
    font-weight: 600;
    color: var(--on-primary);
    text-box-edge: text;
}

/* ── Секции главной ── */
.lib-h2-row {
    padding: 18px 0px 10px 0px;
}
.lib-h2 {
    font-size: 17px;
    font-weight: 700;
    color: var(--text);
}
.lib-more {
    font-size: 13px;
    font-weight: 600;
    color: var(--primary);
}
.lib-more-icon {
    icon-size: 18px;
    color: var(--primary);
}

/* ── Карточка ── */
.lib-card {
    padding: 6px;
    border-radius: 16px;
}
.lib-card:hover {
    background-color: var(--surface-hover);
}
.lib-card-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--text);
}
.lib-card-sub {
    font-size: 12px;
    color: var(--text-muted);
}
.lib-card-kind {
    icon-size: 14px;
    color: var(--text-subtle);
}
.lib-progress {
    height: 4px;
    margin: 4px 0px 0px 0px;
    border-radius: 2px;
    color: var(--primary);
    background-color: var(--border-soft);
}

/* Обложки 3:4. */
.lib-cover {
    border-radius: 12px;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.16), 0 6px 18px rgba(0, 0, 0, 0.10);
}
.lib-cover.book {
    border-radius: 6px 12px 12px 6px;
    background-color: var(--bg-panel);
}
.lib-cover-book-blank {
    background-color: var(--surface-selected);
}
.lib-cover-glyph {
    icon-size: 44px;
    color: var(--text-subtle);
}
.lib-cover.notebook {
    border-radius: 6px 14px 14px 6px;
}
.lib-spine {
    width: 14px;
    background-color: #2B2F3A;
}
.lib-spine.dark {
    background-color: rgba(0, 0, 0, 0.28);
}
.lib-paper {
    background-color: #FFFFFF;
}
.lib-nb-cover {
    padding: 16px 14px;
}
.lib-nb-label {
    padding: 10px 12px;
    border-radius: 8px;
    background-color: rgba(255, 255, 255, 0.92);
}
.lib-nb-label-text {
    font-size: 13px;
    font-weight: 600;
    color: #1C1D22;
    text-align: center;
}
/* Тона обложек блокнотов. */
.tone0 { background-color: #3F6FB5; }
.tone1 { background-color: #2F8F7A; }
.tone2 { background-color: #C2563A; }
.tone3 { background-color: #7A5BB5; }
.tone4 { background-color: #C8963A; }
.tone5 { background-color: #3A3F4B; }
.tone6 { background-color: #B5476B; }
.tone7 { background-color: #4E8A3E; }

.lib-cover.drawing {
    background-color: #FFFFFF;
}
.lib-canvas {
    background-color: #F6F6F3;
}
.lib-cover.doc {
    padding: 14px 14px;
    background-color: #FFFFFF;
    border: 1px solid var(--border-soft);
}
.lib-doc-h {
    font-size: 12px;
    font-weight: 700;
    color: #1C1D22;
}
.lib-doc-text {
    font-size: 9px;
    line-height: 1.45;
    color: #50545C;
}
.lib-cover.folder {
    background-color: var(--primary-soft);
    box-shadow: 0 0 0 rgba(0, 0, 0, 0);
}
.lib-folder-glyph {
    icon-size: 64px;
    color: var(--primary);
}
.lib-folder-count {
    font-size: 12px;
    font-weight: 600;
    color: var(--primary);
}

/* ── Список ── */
.lib-list {
    border-radius: 16px;
    border: 1px solid var(--border-soft);
    background-color: var(--bg-panel);
}
.lib-row {
    min-height: 64px;
    padding: 8px 10px 8px 14px;
    border-bottom: 1px solid var(--border-soft);
}
.lib-row:hover {
    background-color: var(--surface-hover);
}
.lib-row-badge {
    width: 40px;
    height: 40px;
    border-radius: 12px;
    background-color: var(--surface-selected);
}
.lib-row-icon {
    icon-size: 22px;
    color: var(--primary);
}

/* ── Быстрое создание ── */
.lib-quick {
    height: 64px;
    padding: 0px 16px;
    border-radius: 16px;
    border: 1px dashed var(--border-strong);
}
.lib-quick:hover {
    background-color: var(--surface-hover);
}
.lib-quick-badge {
    width: 38px;
    height: 38px;
    border-radius: 12px;
    background-color: var(--primary-soft);
}
.lib-quick-icon {
    icon-size: 22px;
    color: var(--primary);
}
.lib-quick-text {
    font-size: 14px;
    font-weight: 600;
    color: var(--text);
    text-box-edge: text;
}

/* ── Пусто ── */
.lib-empty {
    padding: 72px 24px;
}
.lib-empty-badge {
    width: 88px;
    height: 88px;
    border-radius: 44px;
    background-color: var(--primary-soft);
}
.lib-empty-icon {
    icon-size: 42px;
    font-size: 42px;
    color: var(--primary);
}
.lib-empty-title {
    font-size: 18px;
    font-weight: 700;
    color: var(--text);
}
.lib-empty-hint {
    font-size: 14px;
    color: var(--text-muted);
    text-align: center;
}

/* ── Открытая страница ── */
.lib-openbar {
    min-height: 56px;
    padding: 6px 10px 6px 8px;
    background-color: var(--bg-shell);
    border-bottom: 1px solid var(--border-soft);
}
.lib-open-title {
    font-size: 16px;
    font-weight: 700;
    color: var(--text);
}
.lib-open-sub {
    font-size: 12px;
    color: var(--text-muted);
}
.lib-open-body {
    background-color: var(--bg-shell);
}

/* ── Диалог ── */
.lib-dialog {
    width: 420px;
    padding: 22px;
    border-radius: 20px;
    background-color: var(--bg-panel);
    border: 1px solid var(--border-soft);
    box-shadow: var(--glass-shadow);
}
.lib-dialog-title {
    font-size: 18px;
    font-weight: 700;
    color: var(--text);
}
.lib-dialog-btn {
    height: 40px;
    padding: 0px 20px;
    border-radius: 20px;
    color: var(--text);
}
.lib-dialog-btn.primary {
    background-color: var(--primary);
    color: var(--on-primary);
}
