/* syn-terminal. Палитра (--bg, --fg, --accent, …) — из [appearance],
   цвета терминала (--term-*, --titlebar, --tab-active) — из схемы (state::theme). */

.grow { flex-grow: 1; }
Text { color: var(--fg); font-size: 14px; }

.window { color: var(--term-fg); }

/* Своя рамка окна: радиус — [decorations] corner_radius, как у окон
   композитора; развёрнутое и полноэкранное — без скругления и обводки. */
.window-frame {
    border-radius: var(--window-radius);
    border: 1px solid var(--divider);
}
.window-frame:window-maximized, .window-frame:window-fullscreen {
    border-radius: 0px;
    border-width: 0px;
}

/* ── Заголовок с вкладками ─────────────────────────────────────── */

.titlebar {
    height: 42px;
    background: var(--titlebar);
    padding: 6px 0px 0px 6px;
}
.drag-space { flex-grow: 1; min-width: 40px; }
.tb-tools { padding: 0px 2px 6px 4px; align-items: center; }
.window-controls { padding: 0px 4px 6px 4px; }

SystemWindowControls {
    color: var(--term-fg);
    background-color: var(--term-hover);
}

.tab {
    height: 36px;
    padding: 0px 6px 0px 12px;
    border-radius: 9px 9px 0px 0px;
    align-items: center;
    transition: background 100ms ease;
    &:hover { background: var(--term-hover); }
}
.tab.current {
    background: var(--tab-active);
    &:hover { background: var(--tab-active); }
}
.tab-head { flex-shrink: 1; min-width: 0px; overflow: hidden; }
.tab-icon { color: var(--term-muted); font-size: 16px; icon-size: 16px; }
.tab.current .tab-icon { color: var(--term-cursor); }
.tab-icon.busy { color: var(--term-cursor); font-size: 10px; icon-size: 10px; }
.tab-title { font-size: 12px; color: var(--term-muted); flex-shrink: 1; }
.tab.current .tab-title { color: var(--term-fg); }

.tab-close {
    width: 22px;
    height: 22px;
    border-radius: 6px;
    justify-content: center;
    align-items: center;
    &:hover { background: var(--term-pressed); }
}
.tab-close-icon { font-size: 14px; icon-size: 14px; color: var(--term-muted); }

.new-tab, .tb-btn {
    width: 30px;
    height: 30px;
    border-radius: 7px;
    justify-content: center;
    align-items: center;
    transition: background 100ms ease;
    &:hover { background: var(--term-hover); }
    &:pressed { background: var(--term-pressed); }
}
.new-tab { margin: 0px 0px 3px 4px; }
.ib-icon { font-size: 18px; icon-size: 18px; color: var(--term-fg); }

/* ── Терминал ──────────────────────────────────────────────────── */

.term-area { background: var(--term-bg); }
.term {
    color: var(--term-fg);
    background-color: transparent;
    accent-color: var(--term-cursor);
    padding: 6px 4px 4px 10px;
}

/* ── Телефон ───────────────────────────────────────────────────── */

.phone-bar { background: var(--titlebar); padding: 6px 6px 6px 6px; }
.window.phone .tab {
    height: 34px;
    border-radius: 9px;
    padding: 0px 4px 0px 10px;
}
.window.phone .tab-title { font-size: 13px; }
.window.phone .tb-btn { width: 40px; height: 40px; border-radius: 20px; }
.window.phone .ib-icon { font-size: 22px; icon-size: 22px; }

.xkeys { background: var(--titlebar); padding: 6px 6px 8px 6px; }
.xkey {
    flex-grow: 1;
    height: 40px;
    border-radius: 9px;
    background: var(--term-hover);
    justify-content: center;
    align-items: center;
    transition: background 80ms ease;
    &:pressed { background: var(--term-pressed); }
}
.xkey-text { font-size: 13px; font-weight: 600; color: var(--term-fg); }
.xkey-icon { font-size: 20px; icon-size: 20px; color: var(--term-fg); }

/* ── Настройки ─────────────────────────────────────────────────── */

.set-scrim { background: var(--scrim); }
.set-panel { width: 400px; max-width: 94%; background: var(--bg); border-radius: 18px 0px 0px 18px; box-shadow: -6px 0px 24px #00000066; }
.set-head { padding: 14px 8px 8px 20px; }
.set-title { font-size: 20px; font-weight: 600; }
.set-close { color: var(--fg); }
.set-body { padding: 4px 14px 24px 14px; }
.set-card { padding: 14px; border-radius: 14px; background: var(--surface); }
.set-sec-icon { font-size: 20px; icon-size: 20px; color: var(--accent); }
.set-sec-title { font-size: 15px; font-weight: 600; }
.set-row-title { font-size: 14px; }
.set-value { font-size: 13px; color: var(--muted); }
.set-hint { font-size: 12px; color: var(--muted); }
.set-field { font-size: 13px; }

.swatch {
    padding: 6px;
    border-radius: 12px;
    border: 2px solid transparent;
    transition: background 100ms ease, border-color 100ms ease;
    &:hover { background: var(--hover); }
}
.swatch.selected { border-color: var(--accent); }
.sw-preview { padding: 10px 10px; border-radius: 8px; border: 1px solid var(--border); }
.sw-text { font-family: monospace; font-size: 13px; }
.sw-dot { width: 12px; height: 12px; border-radius: 6px; }
.sw-name { font-size: 12px; }
.sw-check { font-size: 16px; icon-size: 16px; color: var(--accent); }

.kbd { padding: 3px 8px; border-radius: 6px; background: var(--hover); border: 1px solid var(--border); }
.kbd-text { font-size: 12px; font-family: monospace; }
