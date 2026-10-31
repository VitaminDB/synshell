/* syn-dlss — DLSS 5. Палитра (--bg, --fg, --accent, …) — из [appearance], свои переменные — в main.rs. */

.grow { flex-grow: 1; }
.ic { icon-size: 20px; icon-color: var(--muted); }
Text { color: var(--fg); font-size: 14px; }
.hint { font-size: 12px; color: var(--muted); }
.page { padding: 4px 20px 24px 20px; }

/* ── Окно ─────────────────────────────────────────────────────────── */

.window { background: var(--content); color: var(--fg); }
.window-frame { border-radius: var(--window-radius); border: 1px solid var(--border); }
.window-frame:window-maximized, .window-frame:window-fullscreen { border-radius: 0px; border-width: 0px; }
.body { background: var(--content); }
.head { padding: 16px 20px 12px 20px; background: var(--content); }

.titlebar { height: 46px; background: var(--titlebar); }
.tb-left { height: 46px; padding: 0px 10px 0px 12px; align-items: center; }
.drag-wrap { height: 46px; }
.drag-space { height: 46px; flex-grow: 1; min-width: 40px; align-items: center; justify-content: center; }
.tb-title { font-size: 13px; font-weight: 600; color: var(--fg); max-width: 460px; }
.window-controls { height: 46px; padding: 0px 4px; align-items: center; }
SystemWindowControls { color: var(--fg); background-color: var(--hover); }

.brand { width: 28px; height: 28px; border-radius: 9px; background: linear-gradient(135deg, var(--accent), var(--accent-2)); justify-content: center; align-items: center; }
.brand-ic { icon-color: #ffffff; icon-size: 18px; }
.brand-title { font-size: 14px; font-weight: 600; color: var(--fg); }

/* ── Состояние ────────────────────────────────────────────────────── */

.status-card { padding: 16px; border-radius: 16px; background-color: var(--card); border: 1px solid var(--border); box-shadow: 0 1px 3px var(--shadow); }
.status-title { font-size: 15px; font-weight: 600; color: var(--fg); }
.dot { width: 10px; height: 10px; border-radius: 5px; }
.dot.ok { background-color: #43a047; box-shadow: 0 0 8px #43a04780; }
.dot.idle { background-color: #fb8c00; }
.dot.off { background-color: #00000000; border: 1.5px solid var(--muted); }
.dot.bad { background-color: #e53935; }
.spin { color: var(--accent); }

.banner { padding: 12px 14px; border-radius: 12px; background-color: var(--content); border: 1px solid var(--warning); }
.banner-title { font-size: 13px; font-weight: 600; color: var(--fg); }
.banner-info { border-color: var(--accent); }
.info-ic { icon-color: var(--accent); icon-size: 22px; }
.warn-ic { icon-color: var(--warning); icon-size: 22px; }

/* ── Вкладки ──────────────────────────────────────────────────────── */

.tabs { padding: 4px; border-radius: 12px; background-color: var(--card); border: 1px solid var(--border); }
.tab { padding: 7px 14px; border-radius: 9px; transition: background-color 120ms ease-out; }
.tab:hover { background-color: var(--hover); }
.tab.on { background-color: var(--accent-dim); }
.tab-text { font-size: 13px; color: var(--muted); }
.tab.on .tab-text { color: var(--fg); font-weight: 600; }

/* ── Группы и строки ──────────────────────────────────────────────── */

.group-title { color: var(--fg); font-size: 14px; font-weight: 600; padding: 0px 4px; }
.group-card { background: var(--card); border: 1px solid var(--border); border-radius: 14px; box-shadow: 0 1px 3px var(--shadow); }
.row-sep { height: 1px; background: var(--border); }
.setting-row { padding: 12px 16px; min-height: 52px; }
.setting-row-wide { padding: 12px 16px; }
.row-text { flex-grow: 1; }
.row-label { color: var(--fg); font-size: 13px; }
.row-hint { color: var(--muted); font-size: 12px; }

.code { padding: 8px 10px; border-radius: 8px; background-color: var(--content); border: 1px solid var(--border); }
.code-text { font-family: monospace; font-size: 12px; color: var(--fg); }

Dropdown, SpinBox {
    background: var(--input-bg);
    color: var(--fg);
    accent-color: var(--accent);
    border: 1px solid var(--border);
    border-radius: 8px;
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
    font-size: 12px;
}

PopupMenu, ListView { background: var(--menu-bg); color: var(--fg); }

/* ── Кнопки ───────────────────────────────────────────────────────── */

.btn { padding: 9px 14px; border-radius: 10px; background-color: var(--surface); color: var(--fg); font-size: 13px; icon-color: var(--fg); transition: background-color 120ms ease-out; }
.btn:hover { background-color: var(--hover); }
.btn:pressed { background-color: var(--pressed); }
.btn.primary { background: linear-gradient(135deg, var(--accent), var(--accent-2)); color: var(--accent-fg); icon-color: var(--accent-fg); }
.btn.primary:hover { background: linear-gradient(135deg, var(--accent-2), var(--accent)); }
.btn.ghost { background-color: #00000000; border: 1px solid var(--border); }
.btn.ghost:hover { background-color: var(--hover); }
.btn.flat { background-color: #00000000; icon-color: var(--muted); }
.btn.flat:hover { background-color: var(--hover); }
.btn.round { padding: 8px; border-radius: 999px; }
.btn.tb-btn { padding: 8px; icon-size: 20px; icon-color: var(--muted); }
.btn.tb-btn:hover { icon-color: var(--fg); }

TextField.field { background-color: var(--input-bg); border: 1px solid var(--border); border-radius: 10px; padding: 9px 12px; font-size: 13px; color: var(--fg); }
TextField.field:focus { border-color: var(--accent); }

/* ── Диалоги и уведомления ────────────────────────────────────────── */

.scrim { width: 100%; height: 100%; background-color: var(--scrim); }
.dialog-wrap { width: 100%; height: 100%; padding: 24px; }
.dialog-card { width: 460px; padding: 20px; border-radius: 18px; background-color: var(--menu-bg); border: 1px solid var(--border); box-shadow: 0 16px 40px var(--shadow); }
.card-title { font-size: 15px; font-weight: 600; color: var(--fg); }
.toast-wrap { width: 100%; height: 100%; padding: 0px 24px 28px 24px; }
.toast { padding: 10px 16px; border-radius: 12px; background-color: var(--menu-bg); border: 1px solid var(--border); box-shadow: 0 8px 24px var(--shadow); max-width: 560px; }
.toast-text { font-size: 13px; color: var(--fg); }

/* SegmentedButton — без правила виджет рисует белую подложку и чужой синий акцент. */
SegmentedButton { background-color: var(--input-bg); color: var(--fg); border-width: 1px; border-color: var(--border); border-radius: 10px; accent-color: var(--accent); font-size: 13px; }
.link { color: var(--accent); }
