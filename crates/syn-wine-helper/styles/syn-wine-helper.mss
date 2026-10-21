/* syn-wine-helper — «Компоненты Wine». Палитра (--bg, --fg, --accent, …) — из [appearance]. */

.root { background-color: var(--bg); }
.grow { flex-grow: 1; }
Text { color: var(--fg); font-size: 14px; }
.muted { color: var(--muted); }
.small { font-size: 13px; }
.page { padding: 22px 18px 16px 18px; }
.footer { padding: 12px 18px 18px 18px; border-top: 1px solid var(--border); background-color: var(--surface); }
.footer.slim { padding: 0px 18px 0px 18px; }

.title { font-size: 20px; font-weight: 600; }
.name { font-size: 15px; font-weight: 600; }
.dlls { font-size: 12px; color: var(--muted); font-family: monospace; }

.hero { width: 52px; height: 52px; border-radius: 16px; background-color: var(--accent); align-items: center; justify-content: center; }
.hero-ic { font-size: 30px; color: var(--accent-fg); }
.badge { width: 36px; height: 36px; border-radius: 12px; background-color: var(--surface-alt); align-items: center; justify-content: center; }
.badge-ic { font-size: 20px; color: var(--accent); }
.ic { font-size: 22px; color: var(--fg); }
.warn-ic { color: #ffb74d; }
.ok-ic { color: #66bb6a; }
.err-ic { color: #ef5350; }
.err-text { color: var(--fg); font-size: 13px; }

.card { padding: 14px; border-radius: 16px; background-color: var(--surface); }
.warn-card { background-color: #ffb74d1f; }
.chip { padding: 2px 8px; border-radius: 999px; background-color: var(--surface-alt); }
.chip.on { background-color: #66bb6a33; }
.chip-text { font-size: 11px; font-weight: 600; color: var(--muted); }
.bar { height: 6px; }

Button {
    background: var(--surface-alt);
    color: var(--fg);
    accent-color: var(--accent);
    border-radius: 999px;
    padding: 9px 18px;
    font-size: 14px;
    transition: background 120ms ease;
    &:hover { background: var(--pressed); }
    &:pressed { background: var(--hover); }
    &:disabled { opacity: 0.5; }
}
Button.primary {
    background: var(--accent);
    color: var(--accent-fg);
    &:hover { background: var(--accent-hover); }
}
Button.flat { background: transparent; }
Button.round { padding: 8px; }

/* ── Своя рамка окна: заголовок со знаком, перетаскивание, кнопки окна ── */
.window-frame { border-radius: var(--window-radius); border: 1px solid var(--border); background-color: var(--bg); }
.window-frame:window-maximized, .window-frame:window-fullscreen { border-radius: 0px; border-width: 0px; }
.titlebar { height: 46px; background: var(--titlebar); }
.tb-left { height: 46px; padding: 0px 10px 0px 12px; align-items: center; }
.drag-wrap { height: 46px; }
.drag-space { height: 46px; flex-grow: 1; min-width: 40px; align-items: center; justify-content: center; }
.tb-title { font-size: 13px; font-weight: 600; color: var(--muted); max-width: 260px; }
.window-controls { height: 46px; padding: 0px 4px; align-items: center; }
SystemWindowControls { color: var(--fg); background-color: var(--hover); }
.brand { width: 28px; height: 28px; border-radius: 9px; background: linear-gradient(135deg, var(--accent), var(--accent-2)); justify-content: center; align-items: center; }
.brand-ic { font-size: 18px; color: #ffffff; }
.brand-title { font-size: 14px; font-weight: 600; color: var(--fg); }
