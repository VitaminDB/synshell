/* syn-maps — «Карты». Палитра (--bg, --fg, --accent, …) — из [appearance]. */

.grow { flex-grow: 1; }
Text { color: var(--fg); font-size: 14px; }
.muted { color: var(--muted); }

/* своё окно: заголовок над картой, рамка */
.window-frame { border-radius: var(--window-radius); border: 1px solid var(--border); background-color: var(--bg); }
.window-frame:window-maximized, .window-frame:window-fullscreen { border-radius: 0px; border-width: 0px; }
.titlebar { height: 44px; background-color: var(--bg); border-bottom: 1px solid var(--border); }
.tb-drag { height: 44px; }
.tb-drag-space { flex-grow: 1; min-width: 40px; height: 44px; padding: 0px 14px; align-items: center; }
.tb-mark { width: 26px; height: 26px; border-radius: 8px; padding: 4px; background: var(--accent); }
.tb-mark-icon { font-size: 18px; color: #ffffff; }
.tb-title { font-size: 14px; font-weight: 600; }
.tb-sub { font-size: 13px; color: var(--muted); }
.window-controls { padding: 0px 6px; align-items: center; }
SystemWindowControls { color: var(--fg); background-color: var(--hover); }

/* слои «Светлая» и «Тёмная» — плитки OpenStreetMap под фильтром */
.tiles-light { filter: grayscale(0.6) brightness(1.06) contrast(0.92); }
.tiles-dark { filter: invert(1) hue-rotate(180deg) brightness(0.9) contrast(0.85) saturate(0.6); }

/* карточки поверх карты */
.card { background: var(--bg); border-radius: 16px; box-shadow: 0px 2px 10px rgba(0, 0, 0, 0.28); }
.top { padding: 10px 10px 0px 10px; }
.top-desktop { width: 400px; }
.search { padding: 2px 6px 2px 2px; }
.search-field { font-size: 16px; }
.results { max-height: 340px; padding: 6px; }
.results-desktop { max-height: 520px; }
.desktop { }

.ib { width: 44px; height: 44px; border-radius: 22px; padding: 10px; transition: background-color 120ms ease-out, scale 160ms spring(500, 28); }
.ib:hover { background-color: var(--hover); }
.ib:active { background-color: var(--pressed); scale: 0.92; }
.ib-on { background-color: var(--surface); }
.ib-icon { font-size: 24px; color: var(--fg); }
.ib-icon.muted { color: var(--muted); }

.row { padding: 8px 10px; border-radius: 12px; transition: background-color 120ms ease-out; }
.row:hover { background-color: var(--hover); }
.row:active { background-color: var(--pressed); }
.row-icon { font-size: 22px; color: var(--muted); }
.row-icon.active { color: var(--accent); }
.row-title { font-size: 15px; }
.row-title.active { color: var(--accent); font-weight: 600; }
.row-sub { font-size: 12px; color: var(--muted); }

/* кнопки справа внизу */
.fabs { padding: 0px 12px 12px 12px; }
.fab { width: 52px; height: 52px; border-radius: 26px; padding: 14px; background: var(--bg); box-shadow: 0px 2px 8px rgba(0, 0, 0, 0.3); transition: scale 160ms spring(500, 28); }
.fab:active { scale: 0.9; }
.fab-on .ib-icon { color: var(--accent); }
.layers { padding: 6px; }

/* «Местоположение выключено» над кнопками справа */
.loc-wrap { padding: 0px 10px 10px 10px; }
.loc-card { padding: 12px 8px 12px 16px; }
.loc-icon { font-size: 28px; color: var(--accent); }

/* карточка снизу */
.sheet { margin: 0px 10px 10px 10px; padding: 12px 12px 12px 16px; }
.sheet-desktop { width: 400px; }
.sheet-title { font-size: 18px; font-weight: 600; }
.coords { font-size: 12px; color: var(--muted); }
.steps { max-height: 260px; }
.chip { width: 40px; height: 40px; border-radius: 20px; padding: 9px; background: var(--surface); }
.chip-on { background: var(--accent); }
.chip-icon { font-size: 22px; color: var(--fg); }
.chip-icon.on { color: #ffffff; }
.btn { height: 40px; padding: 8px 16px; border-radius: 20px; background: var(--accent); transition: scale 160ms spring(500, 28); }
.btn:active { scale: 0.95; }
.btn-icon { font-size: 20px; color: #ffffff; }
.btn-text { color: #ffffff; font-weight: 600; }

/* в пути */
.nav { padding: 12px 8px 12px 14px; background: var(--accent); }
.nav-icon { font-size: 40px; color: #ffffff; }
.nav-dist { font-size: 22px; font-weight: 700; color: #ffffff; }
.nav-text { font-size: 15px; color: #ffffff; }
.nav .row-sub { color: #ffffff; }
.nav .ib-icon { color: #ffffff; }

.toast { margin: 0px 16px 96px 16px; padding: 10px 14px; border-radius: 12px; background: rgba(30, 30, 30, 0.92); }
.toast-text { color: #ffffff; }
