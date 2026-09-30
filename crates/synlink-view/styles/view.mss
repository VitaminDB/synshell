/* synlink-view — экран другого устройства. Переменные — из [appearance]. */

Text { color: var(--fg); font-size: 13px; }
.grow { flex-grow: 1; }
.icon { icon-size: 20px; icon-color: var(--fg); }

.root { background-color: var(--bg); }

.bar {
    padding: 10px 12px 8px 12px;
    background-color: var(--surface);
    border-bottom-width: 1px;
    border-color: var(--border);
}

.dev-badge { padding: 7px; border-radius: 999px; background-color: var(--accent); box-shadow: 0 3px 12px var(--accent-soft); }
.dev-icon { icon-size: 18px; icon-color: var(--accent-fg); }
.dev-name { font-size: 14px; font-weight: 600; color: var(--fg); }
.dev-state { font-size: 11px; color: var(--muted); }

.chip { padding: 3px 9px 3px 7px; border-radius: 999px; background-color: var(--hover); }
.chip-icon { icon-size: 14px; icon-color: var(--fg); }
.chip-text { font-size: 11px; font-weight: 600; color: var(--fg); }
.chip-usb { background-color: var(--accent); }
.chip-usb .chip-icon { icon-color: var(--accent-fg); }
.chip-usb .chip-text { color: var(--accent-fg); }
.chip-wifi { background-color: var(--success); }
.chip-wifi .chip-icon { icon-color: #ffffff; }
.chip-wifi .chip-text { color: #ffffff; }

.tool {
    padding: 7px;
    border-radius: 12px;
    background-color: var(--surface-alt);
    transition: background-color 140ms ease-out, scale 120ms ease-out;
}
.tool:hover { background-color: var(--hover); }
.tool:active { scale: 0.92; background-color: var(--accent-soft); }
.tool-icon { icon-size: 19px; icon-color: var(--fg); }

.stage { background-color: #05070b; }
.screen { background-color: #05070b; }

.wait-icon { icon-size: 40px; icon-color: var(--accent); animation: view-spin 1400ms linear infinite; }
@keyframes view-spin { 0% { rotate: 0deg; } 100% { rotate: 360deg; } }
.wait-text { font-size: 13px; color: var(--muted); text-align: center; }
