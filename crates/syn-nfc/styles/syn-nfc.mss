/* syn-nfc — «NFC». Палитра (--bg, --fg, --accent, …) — из [appearance]. */

.root { background-color: var(--bg); }
.grow { flex-grow: 1; }
Text { color: var(--fg); font-size: 14px; }
.muted { color: var(--muted); }
.small { font-size: 12px; }
.center { text-align: center; }
.page { padding: 16px; }
.center-page { padding: 48px 24px; }
.title { font-size: 18px; font-weight: 600; }
.big-icon { font-size: 64px; color: var(--muted); }
.warn-text { color: #ffb74d; font-size: 13px; }

.tabs { padding: 6px 8px; background-color: var(--surface); }
.tabs-bottom { padding: 6px 8px 10px 8px; }
.tab { padding: 6px 18px; border-radius: 14px; transition: background-color 120ms ease-out; }
.tab-on { background-color: var(--accent-soft); }
.tab-icon { font-size: 22px; color: var(--muted); }
.tab-on .tab-icon { color: var(--accent); }
.tab-label { font-size: 12px; }

.wait { padding: 24px 16px; }
.wait-ring { width: 112px; height: 112px; border-radius: 56px; padding: 28px; background-color: var(--accent-soft); animation: nfc-pulse 1800ms ease-in-out infinite; }
@keyframes nfc-pulse {
    0% { scale: 1.0; }
    50% { scale: 1.06; }
    100% { scale: 1.0; }
}
.wait-icon { font-size: 56px; color: var(--accent); }

.card { padding: 14px; border-radius: 18px; background-color: var(--surface); }
.card-icon { font-size: 28px; color: var(--accent); }
.card-title { font-size: 15px; font-weight: 600; }
.rec { padding: 8px 10px; border-radius: 12px; background-color: var(--surface-alt); }
.rec-icon { font-size: 22px; color: var(--muted); }
.rec-title { font-size: 14px; }

.toast { padding: 10px 16px; background-color: var(--surface-alt); }
.toast-text { font-size: 13px; }
.field { }

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
Button.small { padding: 5px 12px; font-size: 13px; }
TextField {
    background: var(--surface-alt);
    color: var(--fg);
    accent-color: var(--accent);
    caret-color: var(--accent);
    selection-color: var(--accent-soft);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: 8px 12px;
}
SegmentedButton {
    background: var(--surface-alt);
    color: var(--fg);
    accent-color: var(--accent);
    border-color: var(--border);
}
