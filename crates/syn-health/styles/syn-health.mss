/* syn-health — «Спорт и здоровье». Палитра (--bg, --fg, --accent, …) — из [appearance]. */

.root { background-color: var(--bg); }
.grow { flex-grow: 1; }
Text { color: var(--fg); font-size: 14px; }
.muted { color: var(--muted); }
.small { font-size: 12px; }
.center { text-align: center; }
.page { padding: 20px 16px 32px 16px; }
.note { max-width: 420px; }
.title { font-size: 20px; font-weight: 600; }
.ring-value { font-size: 44px; font-weight: 300; }
.stat { padding: 10px; border-radius: 16px; background-color: var(--surface); }
.stat-value { font-size: 20px; font-weight: 600; }
.step-value { min-width: 60px; text-align: center; }
.card { padding: 14px; border-radius: 18px; background-color: var(--surface); }
.card-title { font-size: 15px; font-weight: 600; }
.set-row { padding: 8px 12px; border-radius: 14px; background-color: var(--surface); }

Button {
    background: var(--surface-alt);
    color: var(--fg);
    accent-color: var(--accent);
    border-radius: 999px;
    padding: 8px 16px;
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
