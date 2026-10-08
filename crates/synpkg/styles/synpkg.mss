/* synpkg — «Программы». Переменные палитры (--bg, --fg, --accent…) — из [appearance]. */

.root { background: var(--bg); flex-grow: 1; }
Text { color: var(--fg); font-size: 14px; }
.grow { flex-grow: 1; }
.muted { color: var(--muted); font-size: 13px; }
.empty { padding: 24px 8px; }

.sidebar { width: 220px; padding: 10px 12px 14px 12px; background: var(--sidebar); }
.brand { font-size: 19px; font-weight: 700; }
.brand-icon-box { padding: 7px; border-radius: 12px; background: var(--accent); }
.brand-icon { font-size: 20px; color: var(--accent-fg); }
.badge { padding: 1px 8px; border-radius: 999px; background: var(--accent); }
.badge-text { font-size: 11px; font-weight: 600; color: var(--accent-fg); }
.nav-item { padding: 9px 12px; border-radius: var(--radius-sm); transition: background-color 120ms ease-out; }
.nav-item:hover { background-color: var(--hover); }
.nav-item.active { background-color: var(--accent-soft); }
.nav-icon { color: var(--fg); font-size: 20px; }
.nav-label { font-size: 14px; }
.list-pane { padding: 0px; }
.detail-pane { width: 480px; border-left-width: 1px; border-color: var(--border); background: var(--surface); }
.pane { padding: 16px; flex-grow: 1; }
.detail-empty { padding: 180px 16px 16px 16px; width: 100%; justify-content: center; }
.empty-icon { font-size: 48px; color: var(--muted); }


.pkg-row {
    padding: 10px 12px;
    border-radius: var(--radius);
    background-color: #00000000;
    transition: background-color 120ms ease-out, scale 200ms spring(420, 26);
}
.pkg-row:hover { background-color: var(--hover); }
.pkg-row:active { scale: 0.98; }
.pkg-row-on { background-color: var(--accent-soft); }
.pkg-icon { width: 40px; height: 40px; }
.pkg-glyph-box { width: 40px; height: 40px; border-radius: 10px; background: var(--surface-alt); padding: 8px; }
.pkg-glyph { font-size: 24px; color: var(--accent); }
.pkg-name { font-size: 15px; font-weight: 600; }
.pkg-desc { font-size: 12px; color: var(--muted); }
.pkg-ver { font-size: 11px; color: var(--muted); }

.chip { padding: 1px 8px; border-radius: 999px; background: var(--surface-alt); }
.chip-text { font-size: 10px; font-weight: 600; }
.chip-aur { background: #7e57c2; }
.chip-aur .chip-text { color: #ffffff; }
.chip-repo { background: var(--surface-alt); }
.chip-ok { background: #2e7d32; }
.chip-ok .chip-text { color: #ffffff; }
.chip-warn { background: #ef6c00; }
.chip-warn .chip-text { color: #ffffff; }
.chip-local { background: #546e7a; }
.chip-local .chip-text { color: #ffffff; }
.chip-dep { background: var(--surface-alt); }

.detail { padding: 18px; }
.big-icon { padding: 4px; }
.h1 { font-size: 22px; font-weight: 600; }
.h2 { font-size: 15px; font-weight: 600; }
.desc { font-size: 14px; color: var(--fg); }
.card { padding: 6px 12px; border-radius: var(--radius); background: var(--surface-alt); }
.kv { padding: 6px 0px; }
.k { width: 110px; font-size: 12px; color: var(--muted); }
.v { font-size: 12px; }
.link { color: var(--accent); }
.warn { padding: 12px; border-radius: var(--radius); background: #ef6c0026; border-width: 1px; border-color: #ef6c0080; }
.warn-text { font-size: 12px; }
.code-box { padding: 10px; border-radius: var(--radius-sm); background: #00000040; max-height: 360px; }
.code { font-family: monospace; font-size: 11px; }


.job { padding: 12px; border-radius: var(--radius); background: var(--surface); }
.job-icon { font-size: 22px; }
.job-running { color: var(--accent); }
.job-ok { color: #43a047; }
.job-err { color: var(--danger); }
.job-cancelled { color: var(--muted); }
.job-error { font-size: 12px; color: var(--danger); }
.log { padding: 8px; border-radius: var(--radius-sm); background: #00000040; flex-grow: 1; min-height: 160px; }
.log-line { font-family: monospace; font-size: 11px; color: var(--muted); }

.toast-place { padding: 0px 16px 84px 16px; }
.toast { padding: 10px 16px; border-radius: 999px; background: var(--fg); box-shadow: 0 6px 20px var(--shadow); }
.toast-text { font-size: 13px; color: var(--bg); }

/* Окно пароля polkit */
.auth-scrim { background: #00000099; padding: 16px; width: 100%; height: 100%; justify-content: center; align-items: center; }
.auth-card { padding: 20px; border-radius: var(--radius); background: var(--surface); box-shadow: 0 10px 32px var(--shadow); max-width: 440px; }
.auth-icon { font-size: 26px; color: var(--accent); }
.auth-title { font-size: 17px; font-weight: 600; }
.auth-error { font-size: 13px; color: var(--danger); }

/* Телефон */
.bar { padding: 8px; }
.back { padding: 8px; border-radius: 999px; transition: background-color 120ms ease-out; }
.back:active { background-color: var(--hover); }
.back-icon { font-size: 24px; color: var(--fg); }
.bar-title { font-size: 18px; font-weight: 600; }
.navbar { padding: 6px 4px 10px 4px; background: var(--surface); border-top-width: 1px; border-color: var(--border); }
.nb-item { padding: 2px 6px; }
.nb-pill { padding: 4px 18px; border-radius: 999px; transition: background-color 200ms ease-out; }
.nb-pill-on { background-color: var(--accent-soft); }
.nb-icon { font-size: 22px; color: var(--fg); }
.nb-label { font-size: 11px; }
.nb-dot { width: 6px; height: 6px; border-radius: 3px; background: var(--accent); }

/* Поля и кнопки — в цветах темы (как в «Параметрах») */
TextField {
    background: var(--input-bg);
    color: var(--fg);
    accent-color: var(--accent);
    caret-color: var(--accent);
    selection-color: var(--accent-soft);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: 7px 12px;
    font-size: 14px;
    icon-color: var(--muted);
    &:focus { border-color: var(--accent); }
}
TextField.search { border-radius: 999px; padding: 9px 16px; }
Button {
    background: var(--surface-alt);
    color: var(--fg);
    accent-color: var(--accent);
    border-radius: 999px;
    padding: 7px 16px;
    font-size: 13px;
    transition: background 120ms ease;
    &:hover { background: var(--pressed); }
    &:pressed { background: var(--hover); }
}
Button.primary {
    background: var(--accent);
    color: var(--accent-fg);
    &:hover { background: var(--accent-hover); }
}
Button.danger {
    background: var(--danger);
    color: #ffffff;
    &:hover { background: var(--danger); }
}
Button.small { padding: 5px 12px; font-size: 12px; }
ProgressBar {
    height: 4px;
    background: var(--pressed);
    color: var(--accent);
    accent-color: var(--accent);
    border-radius: 2px;
}
CircularProgress { color: var(--accent); accent-color: var(--accent); }

/* Каталог по категориям */
.cat-tile {
    padding: 12px 10px;
    border-radius: var(--radius);
    background: var(--surface);
    border-width: 1px;
    border-color: var(--border);
    transition: background-color 120ms ease-out, scale 200ms spring(420, 26);
}
.cat-tile:hover { background-color: var(--hover); }
.cat-tile:active { scale: 0.97; }
.cat-icon-box { padding: 8px; border-radius: 12px; background: var(--accent-soft); }
.cat-icon { font-size: 24px; color: var(--accent); }
.cat-label { font-size: 15px; font-weight: 600; }
.cat-head-icon { font-size: 22px; color: var(--accent); }

/* Задачи: лог открытого задания — на всю высоту */
.job-open { flex-grow: 1; }
.job-row { padding: 10px 12px; transition: background-color 120ms ease-out; }
.job-row:hover { background-color: var(--hover); }
.job-expand { font-size: 22px; color: var(--muted); }
.job-others { max-height: 200px; }

/* Очередь, флажки */
.check-box { padding: 2px; border-radius: 6px; }
.check { font-size: 22px; color: var(--muted); }
.check-on { color: var(--accent); }
.pkg-row-q { background-color: var(--accent-soft); }
.pkg-row-rm { background-color: var(--danger-soft); }
.pkg-row-off { opacity: 0.6; }
.chip-q { background: var(--accent); }
.chip-q .chip-text { color: var(--accent-fg); }
.chip-rm { background: var(--danger); }
.chip-rm .chip-text { color: #ffffff; }
.list-head { padding: 2px 4px 6px 4px; }
.qbar {
    margin: 0px 16px 14px 16px;
    padding: 10px 14px;
    border-radius: var(--radius);
    background: var(--surface);
    border-width: 1px;
    border-color: var(--accent);
    box-shadow: 0 6px 20px var(--shadow);
}
.qbar-icon { font-size: 24px; color: var(--accent); }
.qbar-title { font-size: 14px; font-weight: 600; }
.qbar-sub { font-size: 12px; color: var(--muted); max-width: 420px; }
.q-row { padding: 6px 8px; border-radius: var(--radius-sm); }
.pkg-icon-sm { width: 28px; height: 28px; }
.pkg-icon-sm-glyph { width: 28px; height: 28px; padding: 4px; }
.x-btn { padding: 4px; border-radius: 999px; transition: background-color 120ms ease-out; }
.x-btn:hover { background-color: var(--hover); }
.x-icon { font-size: 18px; color: var(--muted); }
Button.queued { background: var(--accent-soft); color: var(--fg); }

/* Фильтры «Установленных» */
.fchip { padding: 5px 14px; border-radius: 999px; border-width: 1px; border-color: var(--border); transition: background-color 120ms ease-out; }
.fchip:hover { background-color: var(--hover); }
.fchip-on { background-color: var(--accent-soft); border-color: var(--accent); }
.fchip-text { font-size: 13px; }
.section-gap { padding-top: 14px; }
.warn-icon { font-size: 22px; color: #ef6c00; }

/* Витрина */
.app-card {
    padding: 14px;
    border-radius: var(--radius);
    background: var(--surface);
    border-width: 1px;
    border-color: var(--border);
    transition: background-color 120ms ease-out, scale 200ms spring(420, 26);
}
.app-card:hover { background-color: var(--hover); }
.app-card:active { scale: 0.98; }
.card-icon { width: 52px; height: 52px; }
.card-icon-glyph { width: 52px; height: 52px; padding: 12px; }
.card-title { font-size: 15px; font-weight: 600; }
.card-desc { height: 34px; }

/* Подробности */
.hero-icon { width: 80px; height: 80px; }
.hero-icon-glyph { width: 80px; height: 80px; padding: 20px; }
.hero-icon-box { padding: 4px; }
.desc-lead { font-size: 15px; color: var(--fg); }
.shots { height: 196px; }
.shot { width: 300px; height: 180px; border-radius: var(--radius); background: var(--surface-alt); overflow: hidden; border-width: 1px; border-color: var(--border); transition: scale 200ms spring(420, 26); }
.shot:hover { scale: 1.02; }
.shot-img { width: 300px; height: 180px; }
.shot-wait { width: 80px; justify-content: center; align-items: center; }

/* Просмотр снимка */
.viewer { background: #000000d9; padding: 20px; width: 100%; height: 100%; }
.viewer-img { flex-grow: 1; }
.viewer-count { font-size: 14px; color: #ffffff; }
.viewer-nav { padding: 10px; border-radius: 999px; background: #ffffff1f; transition: background-color 120ms ease-out; }
.viewer-nav:hover { background-color: #ffffff40; }
.viewer-nav-icon { font-size: 28px; color: #ffffff; }
.viewer-nav-off { width: 48px; }

/* Удаление с зависимостями */
.dlg-card { max-width: 520px; }
.dlg-icon { font-size: 26px; color: var(--danger); }
.dlg-box { padding: 10px 12px; border-radius: var(--radius-sm); background: #00000030; max-height: 160px; }
.dlg-line { font-size: 13px; }

/* Своё окно (рабочий стол) */
.window-frame { border-radius: var(--window-radius); border: 1px solid var(--border); background: var(--bg); }
.window-frame:window-maximized, .window-frame:window-fullscreen { border-radius: 0px; border-width: 0px; }
.window { background: var(--content); flex-grow: 1; }
.titlebar { height: 52px; background: var(--titlebar); padding: 0px 0px 0px 0px; }
.tb-brand { padding: 0px 18px; }
.tb-back { padding: 6px; border-radius: 999px; transition: background-color 120ms ease-out; }
.tb-back:hover { background-color: var(--hover); }
.tb-back-icon { font-size: 22px; color: var(--fg); }
.tb-back-off { width: 34px; }
.tb-search { width: 520px; padding: 0px 12px; }
.tb-drag { height: 52px; }
.tb-drag-space { flex-grow: 1; height: 52px; min-width: 40px; }
.window-controls { padding: 0px 6px 0px 8px; }
.content { background: var(--content); }
.nav-section { font-size: 12px; font-weight: 600; color: var(--muted); padding: 14px 12px 6px 12px; }
.nav-sep { height: 1px; background: var(--border); margin: 8px 6px; }

/* Обзор и категории */
.page { padding: 22px 28px 28px 28px; }
.hero {
    padding: 22px 24px;
    border-radius: var(--radius);
    background: var(--accent-soft);
    border-width: 1px;
    border-color: var(--border);
}
.hero-title { font-size: 24px; font-weight: 700; }
.hero-sub { font-size: 14px; color: var(--muted); }
.shelf-head { padding-top: 10px; }
.more { padding: 4px 6px 4px 12px; border-radius: 999px; transition: background-color 120ms ease-out; }
.more:hover { background-color: var(--hover); }
.more-text { font-size: 13px; color: var(--accent); }
.more-icon { font-size: 18px; color: var(--accent); }

/* Страница программы */
.page-icon { width: 104px; height: 104px; }
.page-icon-glyph { width: 104px; height: 104px; padding: 28px; }
.page-icon-box { padding: 4px; }
.page-title { font-size: 28px; font-weight: 700; }
.page-right { width: 360px; }
.page-left { min-width: 300px; }
