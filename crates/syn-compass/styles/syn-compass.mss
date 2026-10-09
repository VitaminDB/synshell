/* syn-compass — «Компас». Палитра (--bg, --fg, --accent, …) — из [appearance]. */

.root { background-color: var(--bg); }
.grow { flex-grow: 1; }
Text { color: var(--fg); font-size: 14px; }
.muted { color: var(--muted); }
.center { text-align: center; }
.page { padding: 24px 16px 32px 16px; }
.center-page { padding: 24px; }

.heading { font-size: 64px; font-weight: 300; }
.heading-small { font-size: 36px; font-weight: 300; }
.cardinal { font-size: 28px; font-weight: 600; color: var(--accent); padding: 0px 0px 10px 0px; }
.title { font-size: 20px; font-weight: 600; }

.rose { transition: rotate 120ms ease-out; }
.rose-label { }
.rose-n { font-size: 22px; font-weight: 700; color: #ef5350; text-align: center; }
.rose-main { font-size: 20px; font-weight: 600; text-align: center; }
.rose-num { font-size: 12px; color: var(--muted); text-align: center; }

.info { padding: 16px; border-radius: 18px; background-color: var(--surface); min-width: 320px; max-width: 460px; }
.value { font-size: 15px; font-weight: 600; }
.chip { padding: 4px 10px; border-radius: 999px; }
.chip.ok { background-color: #66bb6a33; }
.chip.warn { background-color: #ffb74d40; }
.chip-text { font-size: 12px; }
.warn-text { color: #ffb74d; font-size: 13px; }
.target-text { color: #ffb74d; font-size: 14px; font-weight: 600; }
.small-icon { font-size: 18px; color: var(--muted); }
.big-icon { font-size: 64px; color: var(--muted); }
.calib { max-width: 420px; }

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
}
Button.primary {
    background: var(--accent);
    color: var(--accent-fg);
    &:hover { background: var(--accent-hover); }
}
