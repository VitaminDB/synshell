/* syn-vm — виртуальные машины. Палитра (--bg, --fg, --accent, …) — из [appearance]. */

.root { background-color: var(--bg); }
.grow { flex-grow: 1; }
.grow-2 { flex-grow: 2; }
.grow-3 { flex-grow: 3; }
Text { color: var(--fg); font-size: 14px; }
Icon { icon-color: var(--fg); icon-size: 20px; }
.muted { color: var(--muted); }
.small { font-size: 12px; }
.pad { padding: 8px 10px; }
.h0 { font-size: 28px; font-weight: 700; }
.h1 { font-size: 21px; font-weight: 700; }
.mono { font-family: "monospace"; font-size: 12px; color: var(--fg); }

/* ── Боковая панель ─────────────────────────────────────────── */
.sidebar { padding: 18px 12px 12px 12px; background-color: var(--surface); border-right: 1px solid var(--border); }
.brand { padding: 2px 6px 4px 6px; }
.brand-logo { padding: 8px; border-radius: 13px; background: linear-gradient(135deg, var(--accent), #b77cff); }
.brand-icon { icon-size: 21px; icon-color: #ffffff; }
.brand-title { font-size: 18px; font-weight: 700; }
.search { margin: 0px 2px; }
.nav-home {
    padding: 9px 12px;
    border-radius: 12px;
    background-color: #00000000;
    transition: background-color 140ms ease;
    &:hover { background-color: var(--hover); }
}
.nav-home.active { background-color: var(--surface-alt); }
.nav-home-icon { icon-size: 20px; icon-color: var(--muted); }
.count { padding: 1px 8px; border-radius: 999px; background-color: var(--success); }
.count-label { font-size: 11px; font-weight: 700; color: #ffffff; }
.nav-item {
    padding: 8px 10px;
    border-radius: 14px;
    border: 1px solid #00000000;
    background-color: #00000000;
    transition: background-color 140ms ease;
    &:hover { background-color: var(--hover); }
}
.nav-item.active { background-color: var(--surface-alt); }
.nav-item:active { scale: 0.98; }
.nav-thumb { width: 56px; height: 36px; border-radius: 9px; background-color: #000000; }
.nav-thumb-img { width: 56px; height: 36px; }
.nav-title { font-size: 14px; font-weight: 600; }
.nav-sub { font-size: 12px; color: var(--muted); }
.host { padding: 10px 12px; border-radius: 14px; background-color: var(--surface-alt); }
.host-title { font-size: 13px; font-weight: 600; }
.host.ok .host-icon { icon-color: var(--success); }
.host.warn .host-icon { icon-color: var(--warning); }
.host.bad .host-icon { icon-color: var(--danger); }
.host-icon { icon-size: 20px; }

.dot { width: 8px; height: 8px; border-radius: 999px; background-color: var(--muted); }
.dot.on { background-color: var(--success); }
.dot.paused { background-color: var(--warning); }
.dot.busy { background-color: var(--accent); }

.main { background-color: var(--bg); }

/* ── Кнопки ─────────────────────────────────────────────────── */
.btn {
    padding: 8px 15px;
    border-radius: 999px;
    background-color: var(--surface-alt);
    transition: background-color 120ms ease, scale 220ms spring(420, 26);
    &:hover { background-color: var(--pressed); }
}
.btn:active { scale: 0.95; }
.btn.round { padding: 8px; }
.btn-icon { icon-size: 18px; }
.btn-label { font-size: 13px; font-weight: 600; }
.btn.primary { background-color: var(--accent); &:hover { background-color: var(--accent); opacity: 0.9; } }
.btn.primary .btn-label { color: var(--accent-fg); }
.btn.primary .btn-icon { icon-color: var(--accent-fg); }
.btn.wide { padding: 10px 15px; }
.btn.big { padding: 12px 22px; }
.btn.big .btn-label { font-size: 15px; }
.btn.soft { background-color: var(--surface-alt); }
.btn.ghost { background-color: #00000000; border: 1px solid var(--border); &:hover { background-color: var(--hover); } }
.btn.danger { background-color: var(--danger); }
.btn.danger .btn-label { color: #ffffff; }
.btn.danger .btn-icon { icon-color: #ffffff; }
.btn.danger-soft { background-color: var(--danger-soft); }
.btn.danger-soft .btn-label { color: var(--danger); }
.btn.danger-soft .btn-icon { icon-color: var(--danger); }
.btn.disabled { opacity: 0.6; }
.ibtn {
    padding: 8px;
    border-radius: 999px;
    background-color: #00000000;
    transition: background-color 120ms ease, scale 220ms spring(420, 26);
    &:hover { background-color: var(--hover); }
}
.ibtn:active { scale: 0.9; }
.ibtn-icon { icon-size: 20px; }

.seg { padding: 3px; border-radius: 999px; background-color: var(--surface-alt); }
.seg-item {
    padding: 7px 14px;
    border-radius: 999px;
    background-color: #00000000;
    transition: background-color 160ms ease;
    &:hover { background-color: var(--hover); }
}
.seg-item.active { background-color: var(--accent); }
.seg-item.active .seg-label { color: var(--accent-fg); }
.seg-item.active .seg-icon { icon-color: var(--accent-fg); }
.seg-label { font-size: 13px; font-weight: 600; }
.seg-icon { icon-size: 18px; }

/* ── Страница машины ────────────────────────────────────────── */
.head { padding: 18px 24px 10px 24px; }
.head.narrow { padding: 10px 10px 6px 10px; }
.tabbar { padding: 4px 24px 12px 24px; border-bottom: 1px solid var(--border); }
.tabbar.narrow { padding: 2px 10px 8px 10px; }
.content { background-color: var(--bg); }
.page { padding: 20px 24px 36px 24px; }
.page.narrow { padding: 12px 14px 90px 14px; }
.page.settings { max-width: 980px; }

.status { padding: 4px 10px; border-radius: 999px; background-color: var(--surface-alt); }
.status.on { background-color: #2ec27e26; }
.status.paused { background-color: #ffb02e26; }
.status.busy { background-color: var(--accent-soft); }
.chip-label { font-size: 12px; font-weight: 600; }

.badge { background-color: var(--accent-soft); }
.badge-icon { icon-color: var(--accent); }

.card { padding: 16px 18px; border-radius: 22px; background-color: var(--surface); border: 1px solid var(--border); }
.card-title { font-size: 15px; font-weight: 600; }
.section-head { padding: 0px 0px 8px 0px; }
.divider { height: 1px; background-color: var(--border); }
.row { padding: 12px 2px; }
.row-title { font-size: 14px; font-weight: 600; }
.row-hint { font-size: 12px; color: var(--muted); }
.row-hint.bad { color: var(--danger); }
.value-label { font-size: 13px; font-weight: 600; width: 76px; text-align: right; }
.kv-key { width: 110px; }
.kv-k { font-size: 13px; color: var(--muted); }
.kv-v { font-size: 13px; }

.tile { padding: 12px 14px; border-radius: 18px; background-color: var(--surface); border: 1px solid var(--border); }
.tile-label { font-size: 12px; color: var(--muted); }
.tile-value { font-size: 18px; font-weight: 700; }
.tile-icon { icon-size: 16px; icon-color: var(--muted); }
.stat-value { font-size: 28px; font-weight: 700; }
.chart { color: var(--border); }
.track { color: var(--surface-alt); }

.preview {
    border-radius: 22px;
    background-color: #000000;
    border: 1px solid var(--border);
    transition: scale 260ms spring(380, 28);
}
.preview:active { scale: 0.99; }
.glass-pill { padding: 10px 18px; border-radius: 999px; background-color: #000000a0; backdrop-filter: blur(12px); }
.pill-icon { icon-color: #ffffff; icon-size: 20px; }
.pill-label { color: #ffffff; font-size: 14px; font-weight: 600; }
.big-play {
    width: 76px;
    height: 76px;
    border-radius: 999px;
    background-color: var(--accent);
    justify-content: center;
    align-items: center;
    box-shadow: 0 10px 30px #00000066;
    transition: scale 240ms spring(420, 22);
    &:hover { scale: 1.06; }
}
.big-play:active { scale: 0.94; }
.big-play-icon { icon-size: 42px; icon-color: var(--accent-fg); }
.code { padding: 12px; border-radius: 12px; background-color: var(--surface-alt); }

/* ── Библиотека ─────────────────────────────────────────────── */
.vm-card {
    padding: 10px 10px 12px 10px;
    border-radius: 22px;
    background-color: var(--surface);
    border: 1px solid var(--border);
    transition: background-color 160ms ease, scale 260ms spring(380, 26), box-shadow 200ms ease;
    &:hover { background-color: var(--surface-alt); box-shadow: 0 12px 32px #00000033; }
}
.vm-card:active { scale: 0.985; }
.thumb { border-radius: 15px; background-color: #000000; }
.thumb.placeholder { background-color: var(--surface-alt); }
.card-name { font-size: 15px; font-weight: 700; }
.play {
    width: 38px;
    height: 38px;
    border-radius: 999px;
    background-color: var(--accent);
    justify-content: center;
    align-items: center;
    transition: scale 220ms spring(420, 24);
    &:hover { scale: 1.08; }
}
.play:active { scale: 0.92; }
.play-icon { icon-size: 22px; icon-color: var(--accent-fg); }
.spacer-btn { width: 38px; height: 38px; }
.fab {
    width: 58px;
    height: 58px;
    border-radius: 20px;
    background-color: var(--accent);
    justify-content: center;
    align-items: center;
    box-shadow: 0 10px 28px #00000055;
}
.fab:active { scale: 0.93; }
.fab-icon { icon-size: 28px; icon-color: var(--accent-fg); }

.hero {
    padding: 40px 28px;
    border-radius: 28px;
    background: linear-gradient(135deg, var(--accent-soft), #b77cff22);
    border: 1px solid var(--border);
}
.hero-badge { padding: 18px; border-radius: 26px; background: linear-gradient(135deg, var(--accent), #b77cff); box-shadow: 0 14px 36px #00000044; }
.hero-icon { icon-size: 44px; icon-color: #ffffff; }
.hero-title { font-size: 24px; font-weight: 700; text-align: center; }
.hero-text { font-size: 14px; color: var(--muted); text-align: center; max-width: 520px; }
.feature { padding: 16px; border-radius: 20px; background-color: var(--surface); border: 1px solid var(--border); }

.banner { padding: 12px 16px; border-radius: 16px; background-color: var(--surface-alt); }
.banner.warn { background-color: #ffb02e1f; border: 1px solid #ffb02e55; }
.banner.warn .banner-icon { icon-color: var(--warning); }
.banner.bad { background-color: var(--danger-soft); border: 1px solid var(--danger); }
.banner.bad .banner-icon { icon-color: var(--danger); }
.banner.info { background-color: var(--accent-soft); }
.banner.info .banner-icon { icon-color: var(--accent); }
.banner-icon { icon-size: 22px; }

.empty { padding: 28px 12px; }
.empty-badge { padding: 16px; border-radius: 22px; background-color: var(--surface-alt); }
.empty-icon { icon-size: 34px; icon-color: var(--muted); }
.empty-title { font-size: 16px; font-weight: 600; text-align: center; }
.empty-hint { font-size: 13px; color: var(--muted); text-align: center; max-width: 420px; }

/* ── Экран ──────────────────────────────────────────────────── */
.screen-wrap { background-color: #0b0b0e; }
.screen-wrap.full { background-color: #000000; }
.screen { background-color: #0b0b0e; }
.float-wrap { padding: 10px 0px 0px 0px; }
.float-bar {
    padding: 4px 6px;
    border-radius: 999px;
    background-color: #16161ccc;
    border: 1px solid #ffffff1f;
    backdrop-filter: blur(16px);
    box-shadow: 0 8px 26px #00000066;
    opacity: 0.55;
    transition: opacity 180ms ease;
    &:hover { opacity: 1.0; }
}
.float-bar .ibtn-icon { icon-color: #ffffff; icon-size: 19px; }
.float-bar .ibtn { padding: 7px; &:hover { background-color: #ffffff22; } }
.float-bar.auto .ibtn-icon { icon-color: #ffffff; }
.res-label { font-size: 11px; color: #ffffffaa; padding: 0px 6px; }
.vsep { width: 1px; height: 20px; background-color: #ffffff2a; margin: 0px 4px; }
.menu {
    padding: 6px;
    border-radius: 14px;
    background-color: #1b1b22f0;
    border: 1px solid #ffffff1f;
    box-shadow: 0 10px 30px #00000077;
    min-width: 200px;
}
.menu-item { padding: 8px 12px; border-radius: 9px; &:hover { background-color: #ffffff1c; } }
.menu-label { color: #ffffff; font-size: 13px; }
.veil { background-color: #000000a6; }
.veil-icon { icon-size: 44px; icon-color: #ffffff; }
.veil-title { font-size: 17px; font-weight: 600; color: #ffffff; }
.veil-sub { font-size: 13px; color: #ffffffaa; }

/* ── Снимки ─────────────────────────────────────────────────── */
.snap { padding: 12px 14px; margin: 0px 0px 10px 0px; border-radius: 16px; background-color: var(--surface-alt); }
.tl-dot { width: 14px; height: 14px; margin: 16px 0px 0px 0px; border-radius: 999px; background-color: var(--surface); border: 3px solid var(--muted); }
.tl-dot.last { border: 3px solid var(--accent); background-color: var(--accent); }
.tl-line { width: 2px; flex-grow: 1; background-color: var(--border); }
.tl-line.end { background-color: #00000000; }

/* ── Поля ───────────────────────────────────────────────────── */
TextField {
    background: var(--surface-alt);
    color: var(--fg);
    accent-color: var(--accent);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 9px 12px;
    font-size: 14px;
    &:focus { border-color: var(--accent); }
}
TextField.search { padding: 8px 12px; }
.mono-field { font-family: "monospace"; font-size: 12px; }
Dropdown { padding: 7px 10px; font-size: 13px; color: var(--fg); background-color: var(--surface-alt); border-radius: 10px; }
Slider {
    height: 4px;
    background: var(--pressed);
    color: var(--accent);
    accent-color: var(--accent);
    border-radius: 2px;
}
Toggle {
    width: 42px;
    height: 24px;
    background: var(--pressed);
    color: #ffffff;
    accent-color: var(--accent);
    border-radius: 12px;
    transition: background-color 180ms ease-out;
    &:checked { background: var(--accent); }
}
CircularProgress { color: var(--accent); accent-color: var(--accent); }
.swatch {
    width: 26px;
    height: 26px;
    border-radius: 999px;
    justify-content: center;
    align-items: center;
    border: 2px solid #00000000;
    transition: scale 200ms spring(420, 24);
    &:hover { scale: 1.12; }
}
.swatch.active { border: 2px solid var(--fg); }
.swatch-check { icon-size: 16px; icon-color: #ffffff; }
.tag { padding: 3px 9px; border-radius: 8px; background-color: var(--surface-alt); }
.tag-label { font-size: 11px; font-weight: 700; color: var(--muted); }

/* ── Окна поверх ────────────────────────────────────────────── */
.scrim { background-color: #00000080; }
.dialog {
    width: 460px;
    padding: 22px 22px 18px 22px;
    border-radius: 26px;
    background-color: var(--surface);
    border: 1px solid var(--border);
    box-shadow: 0 24px 64px #00000066;
}
.dialog-title { font-size: 18px; font-weight: 700; }
.dialog-text { font-size: 14px; color: var(--muted); }
.wiz-body { height: fit-content; }
.step { padding: 4px 10px 4px 4px; border-radius: 999px; }
.step-dot { width: 24px; height: 24px; border-radius: 999px; background-color: var(--surface-alt); justify-content: center; align-items: center; }
.step.active .step-dot { background-color: var(--accent); }
.step.active .step-num { color: var(--accent-fg); }
.step.done .step-dot { background-color: var(--accent-soft); }
.step-num { font-size: 12px; font-weight: 700; color: var(--muted); }
.step-check { icon-size: 15px; icon-color: var(--accent); }
.step-label { font-size: 13px; font-weight: 600; color: var(--muted); }
.step.active .step-label { color: var(--fg); }
.step-line { height: 2px; width: 28px; background-color: var(--border); }
.step-line.done { background-color: var(--accent); }
.option {
    padding: 14px;
    border-radius: 18px;
    background-color: var(--surface-alt);
    border: 2px solid #00000000;
    transition: background-color 140ms ease, scale 220ms spring(420, 26);
    &:hover { background-color: var(--hover); }
}
.option:active { scale: 0.98; }
.option.active { border: 2px solid var(--accent); background-color: var(--accent-soft); }
.option-check { icon-size: 22px; icon-color: var(--accent); }
.picked { padding: 12px 14px; border-radius: 16px; border: 1px solid var(--border); }
.list-item { padding: 9px 12px; border-radius: 12px; &:hover { background-color: var(--hover); } }
.list-item.active { background-color: var(--accent-soft); }
.list-icon { icon-size: 18px; icon-color: var(--muted); }

.toast {
    margin: 0px 0px 22px 0px;
    padding: 8px 8px 8px 16px;
    border-radius: 18px;
    background-color: var(--surface);
    border: 1px solid var(--border);
    box-shadow: 0 12px 36px #00000055;
}
.toast-icon { icon-size: 20px; icon-color: var(--success); }
.toast.error .toast-icon { icon-color: var(--danger); }
.toast-text { font-size: 13px; }
