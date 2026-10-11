/* syn-systeminfo — монитор ресурсов. Палитра (--bg, --fg, --accent, …) — из [appearance]. */

.root { background-color: var(--bg); }
.grow { flex-grow: 1; }
Text { color: var(--fg); font-size: 14px; }
Icon { icon-color: var(--fg); }
.muted { color: var(--muted); }
.small { font-size: 12px; }
.empty { padding: 4px 6px; }

/* ── Боковая панель (компьютер) ─────────────────────────────── */
.sidebar { padding: 18px 12px; background-color: var(--surface); border-right: 1px solid var(--border); }
.brand { padding: 2px 6px 6px 6px; }
.brand-logo { padding: 7px; border-radius: 12px; background-color: var(--accent); }
.brand-icon { icon-size: 20px; icon-color: var(--accent-fg); }
.brand-title { font-size: 17px; font-weight: 700; }
.nav-item {
    padding: 8px 10px;
    border-radius: 14px;
    border: 1px solid #00000000;
    background-color: #00000000;
    transition: background-color 140ms ease;
    &:hover { background-color: var(--hover); }
}
.nav-item.active { background-color: var(--surface-alt); }
.nav-title { font-size: 14px; font-weight: 600; }
.nav-value { font-size: 12px; color: var(--muted); }
.main { background-color: var(--bg); }

/* ── Шапка и чипы (телефон) ─────────────────────────────────── */
.phone-head { padding: 14px 16px 6px 16px; }
.phone-title { font-size: 20px; font-weight: 700; }
.chips { padding: 4px 0px 8px 0px; }
.chips-row { padding: 0px 16px; }
.chip {
    padding: 8px 12px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background-color: var(--surface);
    transition: background-color 140ms ease;
}
.chip:active { scale: 0.96; }
.chip-icon { icon-size: 17px; }
.chip-label { font-size: 13px; font-weight: 600; }
.chip-value { font-size: 12px; color: var(--muted); }

/* ── Страница ───────────────────────────────────────────────── */
.page { padding: 18px 24px 32px 24px; }
.page.narrow { padding: 10px 14px 28px 14px; }
.page-head { padding: 0px 0px 2px 0px; }
.h1 { font-size: 24px; font-weight: 700; }
.section { font-size: 15px; font-weight: 600; padding: 6px 2px 0px 2px; }
.card { padding: 16px; border-radius: 22px; background-color: var(--surface); }
.card-title { font-size: 15px; font-weight: 600; }
.badge-icon { }
.hero-value { font-size: 34px; font-weight: 700; }
.hero-sub { font-size: 13px; color: var(--muted); padding: 0px 0px 6px 0px; }
.axis { font-size: 11px; color: var(--muted); }
.chart { color: var(--border); }
.track { color: var(--surface-alt); }
.dot { width: 10px; height: 10px; border-radius: 999px; }
.legend-v { font-size: 13px; font-weight: 600; }

.tile { padding: 12px 14px; border-radius: 18px; background-color: var(--surface); }
.tile-label { font-size: 12px; color: var(--muted); }
.tile-value { font-size: 19px; font-weight: 700; }
.tile-sub { font-size: 11px; color: var(--muted); }
.res-tile {
    padding: 14px;
    border-radius: 18px;
    background-color: var(--surface);
    transition: background-color 140ms ease, scale 200ms spring(420, 26);
    &:hover { background-color: var(--surface-alt); }
}
.res-tile:active { scale: 0.97; }

.kv-k { font-size: 13px; color: var(--muted); min-width: 130px; }
.kv-v { font-size: 13px; }
.ring-value { font-weight: 700; }
.ring-sub { font-size: 11px; color: var(--muted); }

.seg { padding: 3px; border-radius: 999px; background-color: var(--surface-alt); }
.seg-item { padding: 5px 12px; border-radius: 999px; background-color: #00000000; }
.seg-item.active { background-color: var(--accent); }
.seg-item.active .seg-label { color: var(--accent-fg); }
.seg-label { font-size: 12px; font-weight: 600; }

.core { padding: 10px; border-radius: 14px; background-color: var(--surface-alt); }
.core.off { opacity: 0.5; }
.core-name { font-size: 12px; color: var(--muted); }
.core-value { font-size: 14px; font-weight: 700; }
.core-mhz { font-size: 11px; color: var(--muted); }
.sensor { padding: 12px 14px; border-radius: 18px; background-color: var(--surface); }
.temp-value { font-size: 17px; font-weight: 700; }
.num { font-size: 13px; font-weight: 600; text-align: right; }
.cpu-num, .col-num { width: 52px; text-align: right; }
.mem-num, .col-mem { width: 80px; text-align: right; }
.col-user { width: 110px; }
.col-pid { width: 72px; text-align: right; }
.hot { color: #ff7043; }

/* ── Процессы ──────────────────────────────────────────────── */
.toolbar { padding: 14px 24px 4px 24px; }
.toolbar.narrow { padding: 6px 14px 4px 14px; }
TextField {
    background: var(--surface);
    color: var(--fg);
    accent-color: var(--accent);
    border: 1px solid var(--border);
    border-radius: 14px;
    padding: 9px 12px;
    font-size: 14px;
    &:focus { border-color: var(--accent); }
}
.pill {
    padding: 6px 12px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background-color: #00000000;
}
.pill.active { background-color: var(--accent-soft); border: 1px solid var(--accent); }
.pill-label { font-size: 12px; font-weight: 600; }
.pill-sep { width: 1px; height: 20px; background-color: var(--border); }
.toast { padding: 10px 14px; border-radius: 14px; background-color: var(--accent-soft); }
.toast-icon { icon-size: 18px; icon-color: var(--accent); }
.thead { padding: 6px 10px; }
.th { font-size: 11px; font-weight: 600; color: var(--muted); }
.row-item {
    padding: 8px 10px;
    border-radius: 12px;
    background-color: #00000000;
    transition: background-color 120ms ease;
    &:hover { background-color: var(--hover); }
}
.row-item.selected { background-color: var(--surface-alt); }
.row-title { font-size: 14px; font-weight: 600; }
.avatar { width: 36px; height: 36px; border-radius: 11px; }
.avatar-letter { font-size: 16px; font-weight: 700; color: #ffffff; }
.btn {
    padding: 7px 12px;
    border-radius: 999px;
    background-color: var(--surface-alt);
    transition: background-color 120ms ease;
    &:hover { background-color: var(--pressed); }
}
.btn:active { scale: 0.95; }
.btn-icon { icon-size: 16px; }
.btn.round { padding: 8px; }
.btn-label { font-size: 13px; font-weight: 600; }
.btn.danger { background-color: #e53935; }
.btn.danger .btn-label { color: #ffffff; }
.btn.danger .btn-icon { icon-color: #ffffff; }
.btn.danger-soft { background-color: #e5393526; }
.btn.danger-soft .btn-label { color: #ef5350; }
.btn.danger-soft .btn-icon { icon-color: #ef5350; }
