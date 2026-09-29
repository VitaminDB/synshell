/* Проводник synshell. Палитра (--bg, --accent, …) — из [appearance],
   производные (--titlebar, --chrome, --content, --row-selected…) — из
   state::theme_mss. Тема может дописать свой files.mss. */

.grow { flex-grow: 1; }
.dim { color: var(--muted); }

.window {
    background: var(--content);
    color: var(--fg);
    font-size: 13px;
}

.icon {
    color: var(--fg);
    font-size: 18px;
    icon-size: 18px;
}

/* Бокс с заданным размером прижимает ребёнка к левому верхнему углу —
   значки кнопок, картинки ячеек и строки панелей центрируем явно. */
.icon-btn, .tab-close, .new-tab, .big-icon, .tile-icon, .row-icon {
    justify-content: center;
    align-items: center;
}
.address, .cmd-btn, .hcell, .status-bar, .tab, .crumb, .list-header {
    align-items: center;
}

/* ── Заголовок с вкладками ─────────────────────────────────────── */

.titlebar {
    height: 44px;
    background: var(--titlebar);
    padding: 6px 0px 0px 8px;
}

.tabs { padding: 0px; }
.drag-space { flex-grow: 1; min-width: 40px; }
.window-controls { padding: 0px 4px 6px 8px; }

SystemWindowControls {
    color: var(--fg);
    background-color: var(--hover);
}

.tab {
    width: 220px;
    height: 38px;
    padding: 0px 6px 0px 12px;
    border-radius: 8px 8px 0px 0px;
    transition: background 100ms ease;
    &:hover { background: var(--hover); }
}

.tab.current {
    background: var(--chrome);
    &:hover { background: var(--chrome); }
}

.tab-icon { color: var(--accent); font-size: 16px; icon-size: 16px; }
.tab-title { font-size: 12px; color: var(--fg); }

.tab-close {
    width: 24px;
    height: 24px;
    border-radius: 5px;
    &:hover { background: var(--pressed); }
}
.tab-close .icon { font-size: 14px; icon-size: 14px; color: var(--muted); }

.new-tab {
    width: 32px;
    height: 32px;
    margin: 0px 0px 3px 4px;
    border-radius: 6px;
    &:hover { background: var(--hover); }
}
.new-tab .icon { font-size: 18px; icon-size: 18px; }

/* ── Навигация и адрес ─────────────────────────────────────────── */

.nav-bar {
    background: var(--chrome);
    padding: 6px 10px 6px 8px;
}

.icon-btn {
    width: 32px;
    height: 32px;
    border-radius: 6px;
    transition: background 100ms ease;
    &:hover { background: var(--hover); }
    &:pressed { background: var(--pressed); }
}
.icon-btn .icon { font-size: 18px; icon-size: 18px; }
.icon-btn.disabled .icon { color: var(--muted); opacity: 0.45; }
.icon-btn.disabled { &:hover { background: transparent; } }
.icon-btn.toggled { background: var(--accent-soft); }
.icon-btn.toggled .icon { color: var(--accent); }
.icon-btn.small { width: 26px; height: 26px; }
.icon-btn.small .icon { font-size: 16px; icon-size: 16px; }

.address {
    height: 34px;
    background: var(--field);
    border: 1px solid var(--divider);
    border-radius: 6px;
    padding: 0px 8px;
    &:hover { border-color: var(--border); }
}

.address.edit { padding: 0px; border-width: 0px; background: transparent; }

.address-field, .search-field {
    height: 34px;
    background: var(--field);
    color: var(--fg);
    accent-color: var(--accent);
    border: 1px solid var(--divider);
    border-radius: 6px;
    padding: 6px 10px;
    font-size: 13px;
    &:focus { border-color: var(--accent); }
}

.search { width: 300px; }

.crumb-icon { color: var(--muted); font-size: 18px; icon-size: 18px; margin: 0px 2px 0px 2px; }
.crumb-sep { color: var(--muted); font-size: 16px; icon-size: 16px; }
.crumb-more { color: var(--muted); padding: 0px 4px; }

.crumb {
    height: 26px;
    padding: 0px 8px;
    border-radius: 5px;
    &:hover { background: var(--hover); }
}
.crumb-text { font-size: 13px; color: var(--fg); }
.crumb.last .crumb-text { font-weight: 600; }

/* ── Командная панель ──────────────────────────────────────────── */

.command-bar {
    background: var(--chrome);
    padding: 4px 10px 6px 8px;
    border-bottom: 1px solid var(--divider);
}

.cmd-btn {
    height: 34px;
    padding: 0px 10px;
    border-radius: 6px;
    transition: background 100ms ease;
    &:hover { background: var(--hover); }
    &:pressed { background: var(--pressed); }
}
.cmd-btn.disabled .cmd-label { color: var(--muted); }
.cmd-label { font-size: 13px; color: var(--fg); }
.cmd-btn .chevron { font-size: 16px; icon-size: 16px; color: var(--muted); }
.cmd-btn.accent-btn .icon { color: var(--accent); }

.cmd-sep {
    width: 1px;
    height: 22px;
    background: var(--divider);
    margin: 0px 6px;
}

/* ── Боковая панель ────────────────────────────────────────────── */

.body { background: var(--content); }

.sidebar {
    width: 236px;
    background: var(--sidebar);
    border-right: 1px solid var(--divider);
    padding: 8px 6px 8px 8px;
}

.side-title {
    color: var(--muted);
    font-size: 11px;
    font-weight: 600;
    padding: 8px 10px 4px 10px;
}

.side-sep {
    height: 1px;
    background: var(--divider);
    margin: 6px 8px;
}

.place {
    padding: 6px 10px;
    border-radius: 6px;
    transition: background 100ms ease;
    &:hover { background: var(--row-hover); }
}

.place.active {
    background: var(--row-selected);
    &:hover { background: var(--row-selected-hover); }
}

.place-icon { color: var(--accent); font-size: 18px; icon-size: 18px; }
.place-title { font-size: 13px; color: var(--fg); }

.space-bar {
    width: 150px;
    height: 4px;
    margin: 2px 0px 0px 28px;
    border-radius: 2px;
    background: var(--pressed);
}
.space-fill { height: 4px; border-radius: 2px; background: var(--accent); }
.space-bar.full .space-fill { background: var(--danger); }
.space-text { color: var(--muted); font-size: 11px; margin: 0px 0px 0px 28px; }

/* ── Панели файлов ─────────────────────────────────────────────── */

.content { background: var(--content); }

.pane { background: var(--content); border: 1px solid transparent; }
.pane-wrap { background: var(--content); }
.split .pane.active-pane { border-color: var(--accent-soft); }

.list-header {
    height: 32px;
    padding: 0px 18px 0px 14px;
    border-bottom: 1px solid var(--divider);
}

.hcell {
    height: 30px;
    padding: 0px 8px;
    border-radius: 4px;
    &:hover { background: var(--row-hover); }
}
.htext { font-size: 12px; color: var(--muted); }
.hcell.active .htext { color: var(--fg); }
.sort-arrow { font-size: 14px; icon-size: 14px; color: var(--muted); }

ItemView {
    padding: 4px 8px 8px 6px;
    color: var(--muted);
    selection-color: var(--accent);
}

.item {
    border-radius: 4px;
    border: 1px solid transparent;
    transition: background 80ms ease;
    &:hover { background: var(--row-hover); }
}

.item.selected {
    background: var(--row-selected);
    &:hover { background: var(--row-selected-hover); }
}

.item.cursor { border-color: var(--accent-soft); }
.item.selected.cursor { border-color: var(--accent); }
.item.drop { background: var(--accent-soft); border-color: var(--accent); }
.item.cut { opacity: 0.5; }
.item.dim { opacity: 0.7; }

.item.row { padding: 0px 8px; }
.row-icon { margin: 0px 8px 0px 0px; }
.cell { padding: 0px 8px; }
.name-cell { padding: 0px 8px 0px 0px; }
.name { font-size: 13px; color: var(--fg); }
.name.centered { text-align: center; font-size: 12px; }
.cell-text { font-size: 12px; color: var(--muted); }
.col-size .cell-text { text-align: right; }

.item.list-item { padding: 0px 8px; }

.item.tile { padding: 8px 10px; }
.meta { font-size: 12px; color: var(--muted); }

.item.icon-item { padding: 8px 6px 6px 6px; }

.glyph-icon { color: var(--muted); font-size: 20px; icon-size: 20px; }

.rename-field {
    height: 26px;
    background: var(--field);
    color: var(--fg);
    accent-color: var(--accent);
    border: 1px solid var(--accent);
    border-radius: 4px;
    padding: 3px 6px;
    font-size: 13px;
}

.empty-state { padding: 60px 0px 0px 0px; }
.empty-icon { font-size: 48px; icon-size: 48px; color: var(--muted); opacity: 0.6; }
.empty-text { font-size: 14px; color: var(--muted); }

/* ── Строка состояния ──────────────────────────────────────────── */

.status-bar {
    height: 30px;
    background: var(--content);
    padding: 0px 10px 0px 14px;
    border-top: 1px solid var(--divider);
}
.status-text { font-size: 12px; color: var(--fg); }
.status-text.dim { color: var(--muted); }

Slider {
    height: 4px;
    background: var(--pressed);
    color: var(--accent);
    accent-color: var(--accent);
    border-radius: 2px;
}
.zoom-label { width: 52px; height: 20px; align-items: center; }

/* ── Меню, подсказки ───────────────────────────────────────────── */

PopupMenu {
    background: var(--menu-bg);
    color: var(--fg);
    accent-color: var(--accent);
    border-color: var(--border);
    border-radius: 8px;
    font-size: 13px;
}

Tooltip {
    background: var(--surface);
    color: var(--fg);
    border-color: var(--border);
    font-size: 12px;
}

ScrollView { background: transparent; color: var(--muted); }

Button {
    background: var(--surface-alt);
    color: var(--fg);
    accent-color: var(--accent);
    border-radius: 6px;
    padding: 7px 14px;
    font-size: 13px;
    transition: background 100ms ease;
    &:hover { background: var(--pressed); }
}
Button.primary {
    background: var(--accent);
    color: var(--accent-fg);
    &:hover { background: var(--accent); opacity: 0.9; }
}
Button.danger {
    background: var(--danger);
    color: #ffffff;
}
Button.flat { background: transparent; }

TextField {
    background: var(--field);
    color: var(--fg);
    accent-color: var(--accent);
    border: 1px solid var(--divider);
    border-radius: 6px;
    padding: 7px 10px;
    font-size: 13px;
    &:focus { border-color: var(--accent); }
}

Checkbox { color: var(--fg); accent-color: var(--accent); font-size: 13px; }

ProgressBar {
    height: 4px;
    background: var(--pressed);
    color: var(--accent);
    accent-color: var(--accent);
    border-radius: 2px;
}

/* ── Задания, сообщения, диалоги ───────────────────────────────── */

.jobs-wrap { padding: 0px 16px 44px 0px; }

.jobs { width: 380px; }

.job-card {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 12px 14px;
    box-shadow: 0 8px 24px var(--shadow);
}
.job-title { font-size: 13px; font-weight: 600; color: var(--fg); }
.job-error { font-size: 12px; color: var(--danger); }
.conflict { padding: 8px 0px 0px 0px; }
.conflict-title { font-size: 13px; color: var(--fg); }

.link-btn {
    padding: 4px 8px;
    border-radius: 5px;
    &:hover { background: var(--hover); }
}

.toast-wrap { padding: 0px 0px 44px 0px; }

.toast {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 10px 14px;
    box-shadow: 0 8px 24px var(--shadow);
}
.toast.error { border-color: var(--danger); }
.toast.error .icon { color: var(--danger); }
.toast .icon { color: var(--accent); }
.toast-text { font-size: 13px; color: var(--fg); }
.toast-action {
    padding: 4px 10px;
    border-radius: 5px;
    color: var(--accent);
    &:hover { background: var(--hover); }
}

.scrim { background: #00000066; }

.dialog {
    width: 480px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 20px 22px 18px 22px;
    box-shadow: 0 16px 48px var(--shadow);
}
.dialog-title { font-size: 17px; font-weight: 600; color: var(--fg); }
.dialog-text { font-size: 13px; color: var(--fg); }
.prop-name { font-size: 15px; font-weight: 600; color: var(--fg); }
.prop-val { font-size: 13px; color: var(--fg); }

.app-list { height: 300px; }
.app-row {
    padding: 6px 8px;
    border-radius: 6px;
    &:hover { background: var(--row-hover); }
}
