/* syn-steam-setup — первая установка Steam. Палитра (--bg, --fg, --accent, …) — из [appearance]. */

.root { background-color: var(--bg); }
.grow { flex-grow: 1; }
Text { color: var(--fg); font-size: 14px; }
.muted { color: var(--muted); }
.small { font-size: 13px; }
.mono { font-family: monospace; font-size: 12px; }
.page { padding: 22px 18px 16px 18px; }
.footer { padding: 12px 18px 18px 18px; border-top: 1px solid var(--border); background-color: var(--surface); }
.title { font-size: 20px; font-weight: 600; }

.hero { width: 52px; height: 52px; border-radius: 16px; background: linear-gradient(135deg, #1b2838, #2a475e); align-items: center; justify-content: center; }
.hero.warn { background: #ffb74d; }
.hero.err { background: #ef5350; }
.hero-ic { font-size: 30px; color: #ffffff; }
.ic { font-size: 22px; color: var(--fg); }
.card { padding: 14px; border-radius: 16px; background-color: var(--surface); }
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
Button.link { color: var(--accent); padding: 4px 0px; }

/* ── Своя рамка окна ── */
.window-frame { border-radius: var(--window-radius); border: 1px solid var(--border); background-color: var(--bg); }
.window-frame.flat { border-radius: 0px; border-width: 0px; }
.window-frame:window-maximized, .window-frame:window-fullscreen { border-radius: 0px; border-width: 0px; }
.titlebar { height: 46px; background: var(--titlebar); }
.tb-left { height: 46px; padding: 0px 10px 0px 12px; align-items: center; }
.drag-wrap { height: 46px; }
.drag-space { height: 46px; flex-grow: 1; min-width: 40px; }
.window-controls { height: 46px; padding: 0px 4px; align-items: center; }
SystemWindowControls { color: var(--fg); background-color: var(--hover); }
.brand { width: 28px; height: 28px; border-radius: 9px; background: linear-gradient(135deg, #1b2838, #2a475e); justify-content: center; align-items: center; }
.brand-ic { font-size: 18px; color: #ffffff; }
.brand-title { font-size: 14px; font-weight: 600; color: var(--fg); }
