/* «Параметры системы» synshell. Переменные палитры (--bg, --accent, …)
   приходят из [appearance] через Appearance::mss_variables(). */

.grow { flex-grow: 1; }

.root {
    background: var(--bg);
    color: var(--fg);
    font-size: var(--font-size);
}

/* ── Боковая панель ─────────────────────────────────────────────── */

.sidebar {
    width: 264px;
    background: var(--sidebar-bg);
    border-right: 1px solid var(--border);
    padding: 16px 12px 12px 12px;
}

.brand { padding: 2px 6px 6px 6px; }

.brand-logo {
    width: 34px;
    height: 34px;
    border-radius: 10px;
    background: var(--accent);
}

.brand-icon {
    color: var(--accent-fg);
    icon-size: 20px;
    font-size: 20px;
    width: 34px;
    height: 34px;
}

.brand-title {
    color: var(--fg);
    font-size: 17px;
    font-weight: 600;
}

.nav-scroll { background: transparent; }

.nav { padding: 0px 0px 8px 0px; }

.nav-group {
    color: var(--muted);
    font-size: 11px;
    font-weight: 600;
    padding: 12px 10px 4px 10px;
}

.nav-empty {
    color: var(--muted);
    padding: 12px 10px;
}

.nav-item {
    border-radius: 8px;
    padding: 7px 10px;
    /* Подпись — от левого края (без явного размера бокс центрирует ребёнка). */
    min-width: 0px;
    justify-content: flex-start;
    align-items: center;
    transition: background 120ms ease;
    &:hover { background: var(--hover); }
}

.nav-item.active {
    background: var(--accent-soft);
    &:hover { background: var(--accent-soft); }
}

.nav-icon {
    color: var(--muted);
    font-size: 18px;
    icon-size: 18px;
}

.nav-label {
    color: var(--fg);
    font-size: 13px;
}

.nav-item.active .nav-icon { color: var(--accent); }
.nav-item.active .nav-label { font-weight: 600; }

/* ── Основная область ───────────────────────────────────────────── */

.main { background: var(--bg); }

.page-host { background: var(--bg); }

.page-scroll { background: transparent; }

.page-body {
    padding: 28px 36px 40px 36px;
    max-width: 920px;
}

.page-title {
    color: var(--fg);
    font-size: 24px;
    font-weight: 600;
}

.page-subtitle {
    color: var(--muted);
    font-size: 13px;
}

.group-title {
    color: var(--fg);
    font-size: 14px;
    font-weight: 600;
    padding: 0px 4px;
}

.subgroup-title {
    color: var(--muted);
    font-size: 12px;
    font-weight: 600;
    padding: 4px 4px 0px 4px;
}

.group-card {
    background: var(--card-bg);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    box-shadow: 0 1px 3px var(--shadow);
}

.group-card.pad { padding: 10px 12px; }

.row-sep {
    height: 1px;
    background: var(--border);
}

.setting-row {
    padding: 12px 16px;
    min-height: 52px;
}

.setting-row-wide { padding: 12px 16px; }

.row-text { flex-grow: 1; }

.row-label {
    color: var(--fg);
    font-size: 13px;
}

.row-hint {
    color: var(--muted);
    font-size: 12px;
}

.row-hint.pad { padding: 14px 16px; }

.note {
    color: var(--muted);
    font-size: 12px;
    padding: 0px 4px;
}

.error-text {
    color: var(--danger);
    font-size: 12px;
}

/* ── Элементы управления ────────────────────────────────────────── */

Button {
    background: var(--surface-alt);
    color: var(--fg);
    accent-color: var(--accent);
    border-radius: var(--radius-sm);
    padding: 7px 14px;
    font-size: 13px;
    icon-size: 18px;
    transition: background 120ms ease;
    &:hover { background: var(--pressed); }
    &:pressed { background: var(--hover); }
}

Button.primary {
    background: var(--accent);
    color: var(--accent-fg);
    icon-color: var(--accent-fg);
    &:hover { background: var(--accent-hover); }
}

Button.danger {
    color: var(--danger);
    icon-color: var(--danger);
}

Button.small {
    padding: 5px 10px;
    font-size: 12px;
    icon-size: 16px;
}

Button.icon-btn {
    background: transparent;
    color: var(--muted);
    icon-color: var(--muted);
    padding: 6px;
    icon-size: 18px;
    border-radius: 6px;
    &:hover { background: var(--hover); icon-color: var(--fg); }
}

Button.icon-btn.danger {
    icon-color: var(--muted);
    &:hover { background: var(--danger-soft); icon-color: var(--danger); }
}

.icon-btn-space { width: 30px; height: 30px; }

Button.list-btn {
    background: transparent;
    text-align: left;
    justify-content: flex-start;
    &:hover { background: var(--hover); }
}

Button.combo {
    background: var(--input-bg);
    border: 1px solid var(--border);
    font-size: 12px;
    font-weight: 600;
    padding: 6px 10px;
    min-width: 150px;
}

Button.combo.listening {
    border-color: var(--accent);
    color: var(--accent);
    background: var(--accent-soft);
}

.kb-row.disabled .row-label { color: var(--muted); }

TextField {
    background: var(--input-bg);
    color: var(--fg);
    accent-color: var(--accent);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: 7px 10px;
    font-size: 13px;
    &:focus { border-color: var(--accent); }
}

Dropdown, Autocomplete, SpinBox, Combobox, ColorPicker {
    background: var(--input-bg);
    color: var(--fg);
    accent-color: var(--accent);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    font-size: 13px;
}

Dropdown { padding: 7px 10px; }

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

Slider {
    height: 4px;
    background: var(--pressed);
    color: var(--accent);
    accent-color: var(--accent);
    border-radius: 2px;
}

SegmentedButton {
    background: var(--input-bg);
    color: var(--fg);
    accent-color: var(--accent);
    border-color: var(--border);
    border-radius: var(--radius-sm);
    font-size: 13px;
}

PopupMenu, ListView {
    background: var(--menu-bg);
    color: var(--fg);
    accent-color: var(--accent);
    border-color: var(--border);
}

ScrollView { background: transparent; }

.swatch {
    width: 28px;
    height: 28px;
    border-radius: 8px;
    border: 1px solid var(--border);
}

Button.accent-swatch {
    width: 30px;
    height: 30px;
    padding: 0px;
    border-radius: 15px;
    border: 2px solid transparent;
}

Button.accent-swatch.selected {
    border: 3px solid var(--fg);
}

.chip-row {
    background: var(--input-bg);
    border-radius: var(--radius-sm);
    padding: 2px 4px 2px 10px;
}

.btn-side { min-width: 260px; }

/* ── Апплеты панели ─────────────────────────────────────────────── */

.applet-list { padding: 2px; }

.applet-row {
    padding: 4px 4px 4px 8px;
    border-radius: var(--radius-sm);
    &:hover { background: var(--hover); }
}

.applet-num {
    color: var(--muted);
    font-size: 12px;
    width: 24px;
}

.applet-options {
    background: var(--input-bg);
    border-radius: var(--radius-sm);
    margin: 0px 0px 6px 32px;
}

.applet-add { padding: 8px 4px 2px 4px; }

/* ── Превью ─────────────────────────────────────────────────────── */

.preview-frame {
    height: 190px;
    border-radius: var(--radius);
    border: 1px solid var(--border);
}

.preview-desktop { border-radius: var(--radius); }

.preview-layer { padding: 18px 18px 10px 18px; }

.preview-window {
    width: 260px;
    height: 118px;
    margin: 0px 0px 0px 60px;
    box-shadow: 0 6px 18px #00000055;
}

.preview-titlebar { padding: 8px 10px; }

.preview-title { font-size: 11px; }

.preview-dot {
    width: 10px;
    height: 10px;
    border-radius: 5px;
}

.preview-dot.big {
    width: 16px;
    height: 16px;
    border-radius: 8px;
}

.preview-content { padding: 4px 12px; }

.preview-line { height: 6px; width: 180px; border-radius: 3px; }
.preview-line.short { width: 120px; }

.preview-button {
    width: 64px;
    height: 18px;
    margin: 6px 0px 0px 0px;
}

.preview-panel {
    height: 30px;
    padding: 0px 10px;
}

.preview-task {
    width: 44px;
    height: 16px;
    border-radius: 4px;
}

.preview-clock { font-size: 11px; }

.wall-preview {
    height: 220px;
    border-radius: var(--radius);
    border: 1px solid var(--border);
}

.wall-fill { border-radius: var(--radius); height: 218px; }
.wall-image { height: 218px; }

/* ── Баннер ошибки и строка состояния ───────────────────────────── */

.error-banner {
    background: var(--danger-soft);
    border-bottom: 1px solid var(--danger);
    padding: 10px 20px;
}

.error-icon {
    color: var(--danger);
    icon-size: 22px;
    font-size: 22px;
}

.footer {
    background: var(--sidebar-bg);
    border-top: 1px solid var(--border);
    padding: 6px 16px;
}

.toast {
    color: var(--accent);
    font-size: 12px;
}

/* ── О системе ──────────────────────────────────────────────────── */

.about-hero { padding: 8px 4px; }

.about-title {
    color: var(--fg);
    font-size: 22px;
    font-weight: 600;
}

.about-key { width: 150px; }

/* ── Галерея тем ────────────────────────────────────────────────── */

.theme-card {
    width: 256px;
    border-radius: var(--radius);
    border: 2px solid var(--border);
    background: var(--card-bg);
    box-shadow: 0 1px 3px var(--shadow);
    transition: border-color 120ms ease, box-shadow 160ms ease;
    &:hover {
        border-color: var(--accent-soft);
        box-shadow: 0 6px 18px var(--shadow);
    }
}

.theme-card.selected {
    border-color: var(--accent);
    &:hover { border-color: var(--accent); }
}

.theme-card-empty { width: 268px; }

.theme-thumb {
    height: 150px;
    border-radius: 9px 9px 0px 0px;
}

.theme-wall { border-radius: 9px 9px 0px 0px; }

.thumb-layer { padding: 14px 16px 8px 16px; }

.thumb-window {
    width: 170px;
    height: 88px;
    margin: 0px 0px 0px 30px;
    box-shadow: 0 6px 16px #00000066;
}

.thumb-titlebar {
    padding: 5px 7px;
    border-radius: 6px 6px 0px 0px;
}

.thumb-btn { width: 7px; height: 7px; border-radius: 4px; }

.thumb-content { padding: 6px 10px; }

.thumb-line { height: 5px; width: 120px; border-radius: 3px; }
.thumb-line.short { width: 80px; }

.thumb-button { width: 46px; height: 13px; margin: 3px 0px 0px 0px; }

.thumb-panel {
    height: 22px;
    padding: 0px 8px;
}

.thumb-launcher { width: 11px; height: 11px; border-radius: 6px; }
.thumb-task { width: 30px; height: 12px; border-radius: 3px; }
.thumb-clock { font-size: 9px; }

.theme-meta { padding: 10px 12px 12px 12px; }

.theme-name {
    color: var(--fg);
    font-size: 14px;
    font-weight: 600;
}

.theme-variants {
    color: var(--muted);
    font-size: 11px;
}

.theme-check {
    color: var(--accent);
    icon-size: 18px;
    font-size: 18px;
}

.theme-desc {
    color: var(--muted);
    font-size: 12px;
    line-height: 16px;
    height: 32px;
}

.theme-swatches { padding: 4px 0px 0px 0px; }

.theme-swatch {
    width: 16px;
    height: 16px;
    border-radius: 8px;
    border: 1px solid var(--border);
}


/* ─── Узкое окно (телефон): список разделов → страница ─────────────────── */

.phone-root { background: var(--bg); }
.phone-list { padding: 18px 14px 0px 14px; flex-grow: 1; }
.phone-title { font-size: 30px; font-weight: 600; color: var(--fg); padding: 8px 4px 4px 4px; }
.phone-search { border-radius: 999px; }
.phone-nav-item {
    padding: 12px 14px;
    background-color: #00000000;
    transition: background-color 120ms ease-out, scale 200ms spring(420, 26);
}
.phone-nav-item:active { background-color: var(--hover); scale: 0.98; }
.phone-nav-badge { padding: 7px; border-radius: 12px; background: var(--accent-soft); }
.phone-nav-icon { color: var(--accent); font-size: 20px; }
.phone-nav-label { font-size: 16px; color: var(--fg); }
.phone-nav-chevron { color: var(--muted); font-size: 20px; }

.phone-page { background: var(--bg); }
.phone-bar { padding: 10px 8px; }
.phone-back {
    padding: 8px;
    border-radius: 999px;
    transition: background-color 120ms ease-out, scale 200ms spring(420, 26);
}
.phone-back:active { background-color: var(--hover); scale: 0.9; }
.phone-back-icon { color: var(--fg); font-size: 24px; }
.phone-bar-title { font-size: 20px; font-weight: 600; color: var(--fg); }

.page-body-narrow { padding: 4px 12px 32px 12px; }
.page-body-narrow .page-title { font-size: 20px; }
