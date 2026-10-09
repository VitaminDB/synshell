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

/* Своя рамка окна: радиус — [decorations] corner_radius, как у окон
   композитора; развёрнутое и полноэкранное — без скругления и обводки. */
.window-frame {
    border-radius: var(--window-radius);
    border: 1px solid var(--border);
}
.window-frame:window-maximized, .window-frame:window-fullscreen {
    border-radius: 0px;
    border-width: 0px;
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

/* Ширину задаёт разделитель `.side-split` (перетаскивается, хранится в
   сеансе); его линия — вместо правой границы. */
.sidebar {
    background: var(--sidebar);
    padding: 8px 6px 8px 8px;
}

/* Разделители: линия как у остальных границ, при наведении и
   перетаскивании — акцент (зона захвата шире линии). */
.side-split, .split {
    border-color: var(--divider);
    accent-color: var(--accent);
    divider-thickness: 1px;
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
    height: 42px;
    background: var(--content);
    padding: 0px 14px 0px 14px;
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
    border: 2px solid transparent;
    transition: background 100ms ease;
    &:hover { background: var(--pressed); }
    /* Кнопка с фокусом (по умолчанию в диалоге) — Enter нажмёт её. */
    &:focus { border-color: var(--accent); }
}
Button.primary {
    background: var(--accent);
    color: var(--accent-fg);
    &:hover { background: var(--accent); opacity: 0.9; }
    &:focus { border-color: var(--fg); }
}
Button.danger {
    background: var(--danger);
    color: #ffffff;
    &:focus { border-color: var(--fg); }
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

Toggle {
    width: 40px;
    height: 22px;
    background: var(--pressed);
    color: #ffffff;
    accent-color: var(--accent);
    border-radius: 11px;
    transition: background-color 180ms ease-out;
    &:checked { background: var(--accent); }
}

/* ── Телефон (узкое окно) ──────────────────────────────────────── */

.window.phone { font-size: 14px; }

.pbar {
    height: 56px;
    padding: 0px 4px;
    background: var(--chrome);
}
.pbar.selecting { background: var(--accent-soft); }
.pbar-title { font-size: 19px; font-weight: 600; color: var(--fg); padding: 0px 8px; }

.pbtn {
    width: 44px;
    height: 44px;
    border-radius: 22px;
    justify-content: center;
    align-items: center;
    transition: background 120ms ease, scale 200ms spring(420, 26);
    &:pressed { background: var(--pressed); scale: 0.92; }
}
.pbtn .icon { font-size: 22px; icon-size: 22px; }
.pbtn.small { width: 36px; height: 36px; border-radius: 18px; }
.pbtn.small .icon { font-size: 20px; icon-size: 20px; color: var(--muted); }

.psearch {
    height: 42px;
    border-radius: 21px;
    padding: 8px 14px;
    font-size: 15px;
}

.ppath {
    height: 38px;
    padding: 0px 14px 0px 12px;
    background: var(--chrome);
    border-bottom: 1px solid var(--divider);
    align-items: center;
}
.pcrumb-icon { color: var(--accent); font-size: 18px; icon-size: 18px; margin: 0px 2px 0px 0px; }
.pcrumb {
    height: 30px;
    padding: 0px 6px;
    border-radius: 6px;
    align-items: center;
    &:pressed { background: var(--pressed); }
}
.pcrumb.last .crumb-text { font-weight: 600; }

/* Снизу место под кнопку «Создать»: последний файл не прячется под ней. */
ItemView.phone { padding: 6px 6px 92px 6px; scrollbar-width: 4px; }

.item.prow {
    padding: 0px 10px 0px 12px;
    border-radius: 12px;
    &:pressed { background: var(--row-hover); }
}
.prow-icon { justify-content: center; align-items: center; border-radius: 8px; }
.name.pname { font-size: 15px; }
.item.pcell {
    padding: 10px 4px 6px 4px;
    border-radius: 14px;
    &:pressed { background: var(--row-hover); }
}
.item.pcell .name.centered { font-size: 12px; }
.pick { color: var(--muted); font-size: 22px; icon-size: 22px; }
.pick.on { color: var(--accent); }

.pbottom {
    background: var(--chrome);
    border-top: 1px solid var(--divider);
    padding: 6px 4px 8px 4px;
}
.paction {
    padding: 7px 2px 5px 2px;
    border-radius: 14px;
    align-items: center;
    transition: background 120ms ease;
    &:pressed { background: var(--pressed); }
}
.paction .icon { font-size: 22px; icon-size: 22px; }
.paction-label { font-size: 11px; color: var(--fg); }
.paction.disabled .icon { opacity: 0.35; }
.paction.disabled .paction-label { opacity: 0.35; }

.ppaste { padding: 10px 12px 10px 16px; }
.ppaste-icon { color: var(--accent); font-size: 22px; icon-size: 22px; }
.ppaste-title { font-size: 14px; font-weight: 600; color: var(--fg); }

.fab-wrap { padding: 0px 18px 18px 0px; }
.fab {
    width: 56px;
    height: 56px;
    border-radius: 18px;
    background: var(--accent);
    justify-content: center;
    align-items: center;
    box-shadow: 0 6px 18px var(--shadow);
    transition: scale 200ms spring(420, 26);
    &:pressed { scale: 0.92; }
}
.fab .icon { color: var(--accent-fg); font-size: 26px; icon-size: 26px; }

.drawer-wrap {
    /* Высота — от окна (содержащего блока), не от снимка viewport_size при открытии. */
    height: 100%;
    background: var(--sidebar);
    border-radius: 0px 20px 20px 0px;
    box-shadow: 0 0 32px var(--shadow);
}
.drawer { padding: 14px 10px 10px 10px; flex-grow: 1; }
.drawer-head { padding: 6px 8px 10px 8px; }
.drawer-logo {
    width: 40px;
    height: 40px;
    border-radius: 12px;
    background: var(--accent-soft);
    justify-content: center;
    align-items: center;
}
.drawer-logo .icon { color: var(--accent); font-size: 22px; icon-size: 22px; }
.drawer-title { font-size: 20px; font-weight: 600; color: var(--fg); }
.drawer .place { padding: 11px 12px; border-radius: 12px; }
.drawer .place-title { font-size: 15px; }
.drawer .place-icon { font-size: 22px; icon-size: 22px; }
.drawer .space-bar { margin: 4px 0px 0px 32px; }
.drawer .space-text { margin: 2px 0px 0px 32px; font-size: 12px; }
.drawer .side-title { font-size: 12px; padding: 10px 12px 4px 12px; }

.sheet {
    background: var(--surface);
    border-radius: 22px 22px 0px 0px;
    padding: 8px 14px 18px 14px;
    box-shadow: 0 -8px 28px var(--shadow);
}
.sheet-handle {
    width: 36px;
    height: 4px;
    border-radius: 2px;
    background: var(--border);
    margin: 2px 0px 4px 0px;
}
.sheet .side-title { font-size: 12px; padding: 10px 10px 4px 10px; }
.sheet-row {
    padding: 10px 10px;
    border-radius: 12px;
    &:pressed { background: var(--pressed); }
}
.sheet-row .icon { color: var(--muted); font-size: 20px; icon-size: 20px; }
.sheet-row.on .icon { color: var(--accent); }
.sheet-row.on .sheet-label { color: var(--accent); }
.sheet-label { font-size: 15px; color: var(--fg); }
.view-chips { padding: 2px 0px 4px 0px; }
.view-chip {
    height: 44px;
    border-radius: 12px;
    border: 1px solid var(--divider);
    justify-content: center;
    align-items: center;
    &:pressed { background: var(--pressed); }
}
.view-chip .icon { color: var(--muted); font-size: 20px; icon-size: 20px; }
.view-chip.on { background: var(--accent-soft); border-color: var(--accent); }
.view-chip.on .icon { color: var(--accent); }
.view-chip.on .sheet-label { color: var(--accent); }
/* Сообщение — над кнопкой «Создать». */
.phone .toast-wrap { padding: 0px 16px 92px 16px; }
.phone .jobs-wrap { padding: 0px 12px 92px 12px; }
.sheet .side-sep { margin: 6px 4px; }
.phone .empty-state { padding: 120px 24px 0px 24px; }

/* окно выбора файлов портала (synfiles --choose) */
.chooser-bar { padding: 10px 12px 14px 12px; background: var(--surface); border-top-width: 1px; border-color: var(--border); }
.ch-info { font-size: 13px; color: var(--muted); }
.ch-name { width: 100%; }
.ch-chip { padding: 5px 12px; border-radius: 14px; border-width: 1px; border-color: var(--border); }
.ch-chip-on { background: var(--accent-soft); border-color: var(--accent); }
.ch-chip-text { font-size: 12px; color: var(--fg); }
.ch-btn { padding: 9px 16px; border-radius: 20px; background: var(--surface-alt); transition: background-color 120ms ease-out, scale 140ms spring(500, 28); }
.ch-btn:active { scale: 0.95; }
.ch-btn-main { background: var(--accent); }
.ch-btn-text { font-size: 14px; font-weight: 600; color: var(--fg); }
.ch-btn-main .ch-btn-text { color: #ffffff; }
.ch-btn-icon { font-size: 18px; color: #ffffff; }

/* Размеры папок (`[files] dir_sizes`): чип на значке, в списке и плитках. */
.size-chip {
    padding: 1px 7px;
    border-radius: 9px;
    background: var(--accent-soft);
}
.size-chip-text { font-size: 11px; font-weight: 600; color: var(--accent); }
.size-chip.partial { background: var(--hover); }
.size-chip.partial .size-chip-text { color: var(--muted); font-weight: 400; }
.size-chip.on-icon {
    background: var(--surface);
    border: 1px solid var(--accent-soft);
    box-shadow: 0 2px 6px var(--shadow);
    margin: 0px 0px 2px 0px;
}
.size-chip.on-icon.partial { border-color: var(--divider); }
.chip-area { justify-content: center; align-items: center; }
.chip-layer { justify-content: center; align-items: end; }
.dir-size .cell-text { color: var(--accent); }
.dir-size.partial .cell-text { color: var(--muted); }

/* Настройки: панель справа поверх окна. */
.set-scrim { background: #00000059; }
.set-panel {
    height: 100%;
    background: var(--sidebar);
    border-radius: 18px 0px 0px 18px;
    border-left: 1px solid var(--divider);
    box-shadow: -8px 0px 32px var(--shadow);
}
.set-panel-inner { flex-grow: 1; }
.set-head { padding: 16px 10px 12px 18px; }
.set-logo {
    width: 40px;
    height: 40px;
    border-radius: 12px;
    background: var(--accent-soft);
    justify-content: center;
    align-items: center;
}
.set-logo .icon { color: var(--accent); font-size: 22px; icon-size: 22px; }
.set-title { font-size: 20px; font-weight: 600; color: var(--fg); }
.set-body { padding: 2px 14px 24px 14px; }
.set-card {
    padding: 14px;
    border-radius: 14px;
    background: var(--chrome);
    border: 1px solid var(--divider);
}
.set-sec-badge {
    width: 28px;
    height: 28px;
    border-radius: 8px;
    background: var(--accent-soft);
    justify-content: center;
    align-items: center;
}
.set-sec-icon { color: var(--accent); font-size: 18px; icon-size: 18px; }
.set-sec-title { font-size: 15px; font-weight: 600; color: var(--fg); }
.set-row-title { font-size: 13px; color: var(--fg); }
.set-hint { font-size: 12px; color: var(--muted); }
.set-value { font-size: 12px; color: var(--accent); font-weight: 600; }
.set-panel.phone { border-radius: 22px 0px 0px 22px; }
.set-panel.phone .set-row-title { font-size: 15px; }
.set-panel.phone .set-hint { font-size: 13px; }
.set-panel SegmentedButton {
    background: var(--field);
    color: var(--fg);
    border-color: var(--divider);
    accent-color: var(--accent);
}

/* Журнал изменений (synfsd): история в «Свойствах», исключения в настройках. */
.hist-title { font-size: 14px; font-weight: 600; color: var(--fg); }
.hist-title-icon { color: var(--accent); }
.hist-row { padding: 6px 4px; border-radius: 8px; &:hover { background: var(--hover); } }
.hist-what { font-size: 13px; color: var(--fg); }
.hist-exe { font-size: 11px; }
.hist-badge {
    width: 28px;
    height: 28px;
    border-radius: 14px;
    background: var(--accent-soft);
    justify-content: center;
    align-items: center;
}
.hist-badge .icon { font-size: 16px; icon-size: 16px; color: var(--accent); }
.hist-badge.deleted { background: var(--hover); }
.hist-badge.deleted .icon { color: var(--danger); }
.excl-row { padding: 4px 4px 4px 10px; border-radius: 10px; background: var(--field); border: 1px solid var(--divider); }
.excl-icon { font-size: 16px; icon-size: 16px; color: var(--muted); }
.excl-name { font-size: 13px; color: var(--fg); }
