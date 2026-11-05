/* synpass — «Пароли». Переменные палитры (--bg, --fg, --accent…) — из [appearance]. */

.root { background: var(--bg); flex-grow: 1; }
Text { color: var(--fg); font-size: 14px; }
.grow { flex-grow: 1; }
.muted { color: var(--muted); font-size: 13px; text-align: center; }

/* ─── кнопки ─── */
.ibtn { width: 44px; height: 44px; border-radius: 22px; padding: 10px; transition: background-color 120ms ease-out, scale 180ms spring(420, 26); }
.ibtn:hover { background-color: var(--hover); }
.ibtn:active { background-color: var(--pressed); scale: 0.92; }
.ibtn-icon { font-size: 24px; icon-size: 24px; color: var(--fg); icon-color: var(--fg); }
.ibtn-soft .ibtn-icon { color: var(--muted); icon-color: var(--muted); }
.ibtn-accent { background-color: var(--accent-soft); }
.ibtn-accent .ibtn-icon { color: var(--accent); icon-color: var(--accent); }
.ibtn-accent:hover { background-color: var(--accent-soft); opacity: 0.85; }
.ibtn-star .ibtn-icon { color: var(--warning); icon-color: var(--warning); }

.btn {
    height: 44px; padding: 0px 18px; border-radius: 14px;
    background-color: var(--surface-alt); color: var(--fg);
    justify-content: center; align-items: center;
    transition: background-color 120ms ease, scale 180ms spring(420, 26), opacity 120ms ease;
}
.btn:hover { background-color: var(--hover); }
.btn:active { scale: 0.97; }
.btn-icon { font-size: 20px; icon-size: 20px; color: var(--fg); icon-color: var(--fg); }
.btn-label { font-size: 15px; font-weight: 600; color: var(--fg); text-align: center; }
.btn-primary .btn-label, .btn-primary .btn-icon { color: var(--accent-fg); icon-color: var(--accent-fg); }
.btn-tonal .btn-label, .btn-tonal .btn-icon { color: var(--accent); icon-color: var(--accent); }
.btn-danger .btn-label, .btn-danger .btn-icon { color: #ffffff; icon-color: #ffffff; }
.btn-danger-flat .btn-label, .btn-danger-flat .btn-icon { color: var(--danger); icon-color: var(--danger); }
.btn-primary { background-color: var(--accent); color: var(--accent-fg); }
.btn-primary:hover { background-color: var(--accent); opacity: 0.9; }
.btn-tonal { background-color: var(--accent-soft); color: var(--accent); }
.btn-tonal:hover { background-color: var(--accent-soft); opacity: 0.85; }
.btn-flat { background-color: #00000000; color: var(--fg); }
.btn-danger { background-color: var(--danger); color: #ffffff; }
.btn-danger:hover { background-color: var(--danger); opacity: 0.9; }
.btn-danger-flat { color: var(--danger); }
.btn-big { height: 56px; border-radius: 18px; }
.btn-big .btn-label { font-size: 16px; }
.btn-small { height: 34px; padding: 0px 12px; border-radius: 10px; }
.btn-small .btn-label { font-size: 13px; }
.btn-small .btn-icon { font-size: 16px; icon-size: 16px; }
.btn-wide { width: 100%; }
.btn-busy { animation: pulse 900ms ease-in-out infinite; }
@keyframes pulse { 0% { opacity: 1; } 50% { opacity: 0.55; } 100% { opacity: 1; } }

/* ─── поля ─── */
.field {
    min-height: 48px; padding: 12px 14px; border-radius: 14px;
    background-color: var(--input-bg); border-width: 1px; border-color: var(--border);
    color: var(--fg); font-size: 16px; caret-color: var(--accent);
    icon-color: var(--muted);
    transition: border-color 140ms ease;
    &:focus { border-color: var(--accent); }
}
.field-pass { font-family: monospace; font-size: 18px; }
.search { min-height: 44px; padding: 10px 14px; border-radius: 22px; font-size: 15px; }
.notes MultilineTextEdit, MultilineTextEdit.notes {
    background: var(--input-bg); color: var(--fg); accent-color: var(--accent);
    border: 1px solid var(--border); border-radius: 14px; font-size: 15px; padding: 10px 12px;
}
.ed-label { font-size: 12px; font-weight: 600; color: var(--muted); letter-spacing: 0.4px; }

/* ─── сила пароля ─── */
.seg { height: 5px; flex-grow: 1; border-radius: 3px; background-color: var(--surface-alt); transition: background-color 200ms ease; }
.seg-0 { background-color: var(--danger); }
.seg-1 { background-color: var(--danger); }
.seg-2 { background-color: var(--warning); }
.seg-3 { background-color: var(--success); }
.seg-4 { background-color: var(--success); }
.seg-label { font-size: 12px; color: var(--muted); min-width: 92px; text-align: right; }
.seg-label-0, .seg-label-1 { color: var(--danger); }
.seg-label-2 { color: var(--warning); }
.seg-label-3, .seg-label-4 { color: var(--success); }

/* ─── вход ─── */
.gate { padding: 32px 16px; min-height: 100%; width: 100%; }
.gate-card {
    width: 100%; max-width: 440px;
    padding: 30px 26px 24px 26px; border-radius: 26px;
    background-color: var(--card); border-width: 1px; border-color: var(--border);
    box-shadow: 0 24px 64px var(--shadow);
    animation: appear 280ms ease-out-back;
}
@keyframes appear { from { opacity: 0; scale: 0.94; } to { opacity: 1; scale: 1; } }
.badge { width: 84px; height: 84px; border-radius: 42px; background-color: var(--accent-soft); border-width: 1px; border-color: var(--accent); }
.badge-icon { font-size: 42px; icon-size: 42px; color: var(--accent); icon-color: var(--accent); }
.gate-title { font-size: 26px; font-weight: bold; text-align: center; }
.gate-sub { font-size: 14px; color: var(--muted); text-align: center; line-height: 20px; }
.fields { width: 100%; }
.shake-a { animation: shake-a 380ms ease-out; }
.shake-b { animation: shake-b 380ms ease-out; }
.shake-a .field, .shake-b .field { border-color: var(--danger); }
@keyframes shake-a { from { translate-x: 0px; } 20% { translate-x: -10px; } 40% { translate-x: 9px; } 60% { translate-x: -6px; } 80% { translate-x: 3px; } to { translate-x: 0px; } }
@keyframes shake-b { from { translate-x: 0px; } 20% { translate-x: -10px; } 40% { translate-x: 9px; } 60% { translate-x: -6px; } 80% { translate-x: 3px; } to { translate-x: 0px; } }
.q-head { padding: 8px 0px 0px 0px; }
.q-box { padding: 12px 14px; border-radius: 14px; background-color: var(--accent-soft); }
.q-text { font-size: 17px; font-weight: 600; color: var(--fg); }
.gate-error { font-size: 13px; color: var(--danger); text-align: center; }
.gate-or { font-size: 12px; color: var(--muted); text-align: center; padding: 4px 0px; }
.gate-foot { font-size: 12px; color: var(--muted); }
.gate-foot-icon { font-size: 16px; icon-size: 16px; color: var(--muted); icon-color: var(--muted); }

.dev-card { padding: 12px 14px; border-radius: 16px; background-color: var(--surface-alt); transition: background-color 120ms ease; }
.dev-card:hover { background-color: var(--hover); }
.dev-icon { font-size: 26px; icon-size: 26px; color: var(--accent); icon-color: var(--accent); }
.dev-name { font-size: 15px; font-weight: 600; }
.dev-sub { font-size: 12px; color: var(--muted); }
.dev-warn { color: var(--warning); }
.dev-go { font-size: 20px; icon-size: 20px; color: var(--muted); icon-color: var(--muted); }

/* ─── список ─── */
.side { width: 380px; border-right-width: 1px; border-color: var(--border); background: var(--surface); }
.main { background: var(--bg); }
.bar { padding: 10px 10px 6px 14px; min-height: 60px; }
.title { font-size: 26px; font-weight: bold; padding: 4px 4px; }
.bar-title { font-size: 18px; font-weight: 600; }
.tools { padding: 4px 14px 10px 14px; }
.segbtn { height: 34px; padding: 0px 12px; border-radius: 17px; background-color: var(--surface-alt); justify-content: center; align-items: center; transition: background-color 140ms ease; }
.segbtn:hover { background-color: var(--hover); }
.segbtn-on { background-color: var(--accent); }
.segbtn-on:hover { background-color: var(--accent); }
.seg-text { font-size: 13px; font-weight: 600; color: var(--fg); }
.segbtn-on .seg-text { color: var(--accent-fg); }

.chip { height: 32px; padding: 0px 12px; border-radius: 16px; background-color: var(--surface-alt); align-items: center; max-width: 136px; margin: 0px 4px 0px 8px; }
.chip-icon { font-size: 16px; icon-size: 16px; color: var(--muted); icon-color: var(--muted); }
.chip-text { font-size: 12px; color: var(--muted); flex-shrink: 1; }
.chip-ok { background-color: var(--accent-soft); }
.chip-ok .chip-icon, .chip-ok .chip-text { color: var(--accent); icon-color: var(--accent); }
.chip-warn .chip-icon, .chip-warn .chip-text { color: var(--warning); icon-color: var(--warning); }
.chip-busy { animation: pulse 900ms ease-in-out infinite; }

.list { padding: 2px 8px 104px 8px; }
.row { padding: 10px 8px 10px 10px; border-radius: 18px; background-color: #00000000; transition: background-color 120ms ease-out; }
.row:hover { background-color: var(--hover); }
.row:active { background-color: var(--pressed); }
.row-on { background-color: var(--accent-soft); }
.row-on:hover { background-color: var(--accent-soft); }
.row-title { font-size: 16px; font-weight: 600; }
.row-sub { font-size: 13px; color: var(--muted); }
.row-star { font-size: 15px; icon-size: 15px; color: var(--warning); icon-color: var(--warning); }

.empty { padding: 96px 16px 16px 16px; width: 100%; }
.empty-icon { font-size: 56px; icon-size: 56px; color: var(--muted); icon-color: var(--muted); opacity: 0.6; }
.empty-detail { padding: 24px; }
.empty-title { font-size: 18px; font-weight: 600; }

.fab-place { padding: 0px 20px 24px 0px; }
.fab { width: 60px; height: 60px; border-radius: 20px; padding: 16px; background: var(--accent); box-shadow: 0 8px 22px var(--shadow); transition: scale 200ms spring(420, 26); }
.fab:active { scale: 0.92; }
.fab-icon { font-size: 28px; icon-size: 28px; color: var(--accent-fg); icon-color: var(--accent-fg); }

/* ─── кружки ─── */
.av { border-radius: 999px; }
.av-m { width: 46px; height: 46px; border-radius: 15px; }
.av-l { width: 84px; height: 84px; border-radius: 26px; }
.av-text { color: #ffffff; font-weight: bold; text-align: center; }
.av-text-m { font-size: 20px; }
.av-text-l { font-size: 38px; }
.av0 { background-color: #5b8def; }
.av1 { background-color: #e0697b; }
.av2 { background-color: #33a87e; }
.av3 { background-color: #e39a2d; }
.av4 { background-color: #9b6fe3; }
.av5 { background-color: #22a6ba; }
.av6 { background-color: #e57b4a; }
.av7 { background-color: #6f7f99; }

/* ─── запись ─── */
.detail { padding: 6px 20px 40px 20px; width: 100%; max-width: 680px; }
.hero { padding: 4px 0px 2px 0px; }
.hero-title { font-size: 28px; font-weight: bold; text-align: center; }
.hero-sub { font-size: 14px; color: var(--muted); text-align: center; }
.fc { padding: 14px 10px 14px 16px; border-radius: 20px; background-color: var(--card); border-width: 1px; border-color: var(--border); }
.fc-icon { font-size: 22px; icon-size: 22px; color: var(--muted); icon-color: var(--muted); }
.fc-label { font-size: 12px; font-weight: 600; color: var(--muted); letter-spacing: 0.4px; }
.fc-value { font-size: 18px; }
.fc-pass { font-family: monospace; font-size: 22px; letter-spacing: 1px; }
.fc-dots { font-size: 22px; letter-spacing: 2px; color: var(--muted); }
.fc-link { color: var(--accent); }
.fc-notes { font-size: 15px; line-height: 22px; }
.meta { font-size: 12px; color: var(--muted); text-align: center; }

/* ─── крупный просмотр ─── */
.scrim { background-color: var(--scrim); flex-grow: 1; }
.overlay-place { padding: 16px; }
.sheet {
    width: 100%; max-width: 560px;
    padding: 20px 20px 22px 22px; border-radius: 26px;
    background-color: var(--card); border-width: 1px; border-color: var(--border);
    box-shadow: 0 30px 80px var(--shadow);
    animation: appear 240ms ease-out-back;
}
.sheet-big { max-width: 760px; }
.big-title { font-size: 20px; font-weight: bold; }
.big-sub { font-size: 13px; color: var(--muted); }
.bc-scroll { max-height: 440px; }
.bc-grid { padding: 4px 0px; }
.bc { width: 52px; height: 74px; border-radius: 14px; background-color: var(--surface-alt); justify-content: center; align-items: center; }
.bc-alt { background-color: var(--input-bg); border-width: 1px; border-color: var(--border); }
.bc-char { font-family: monospace; font-size: 38px; font-weight: bold; color: var(--fg); text-align: center; }
.bc-upper { color: var(--accent); }
.bc-digit { color: #4c8dff; }
.bc-sym { color: var(--warning); }
.bc-num { font-size: 10px; color: var(--muted); text-align: center; }
.lg { font-size: 12px; color: var(--fg); }
.lg-upper { color: var(--accent); }
.lg-digit { color: #4c8dff; }
.lg-sym { color: var(--warning); }

/* ─── редактор ─── */
.editor { padding: 6px 20px 48px 20px; width: 100%; max-width: 680px; }
.gen { padding: 14px; border-radius: 18px; background-color: var(--surface-alt); }
.gen-len { font-size: 18px; font-weight: bold; min-width: 34px; text-align: center; }
.danger-box { padding: 14px; border-radius: 18px; background-color: var(--surface-alt); border-width: 1px; border-color: var(--danger); }
.danger-text { font-size: 14px; }

/* ─── настройки ─── */
.set-scroll { max-height: 560px; }
.set-title { font-size: 15px; font-weight: 600; }
.set-sub { font-size: 12px; color: var(--muted); line-height: 17px; }

/* ─── всплывающее сообщение ─── */
.toast-place { padding: 0px 16px 100px 16px; }
.toast { padding: 12px 18px; border-radius: 18px; background: var(--fg); box-shadow: 0 8px 24px var(--shadow); max-width: 520px; animation: toast-in 220ms ease-out; }
@keyframes toast-in { from { opacity: 0; translate-y: 12px; } to { opacity: 1; translate-y: 0px; } }
.toast-icon { font-size: 18px; icon-size: 18px; color: var(--bg); icon-color: var(--bg); }
.toast-text { font-size: 14px; color: var(--bg); }
