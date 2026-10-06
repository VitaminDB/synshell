/* syn-video-player — «Видео». Палитра (--bg, --fg, --accent, …) — из [appearance]. */

.grow { flex-grow: 1; }
.root { background: var(--bg); }
Text { color: var(--fg); font-size: 14px; }

/* библиотека */
.header { padding: 12px 12px 8px 16px; }
.title { font-size: 22px; font-weight: 600; }
.ib { width: 44px; height: 44px; border-radius: 22px; padding: 10px; transition: background-color 120ms ease-out, scale 160ms spring(500, 28); }
.ib:hover { background-color: var(--hover); }
.ib:active { background-color: var(--pressed); scale: 0.92; }
.ib-icon { font-size: 24px; color: var(--fg); }

.grid-pad { padding: 4px 12px 24px 12px; }
.card { padding: 6px; border-radius: 14px; transition: background-color 120ms ease-out, scale 160ms spring(500, 28); }
.card:hover { background-color: var(--hover); }
.card:active { background-color: var(--pressed); scale: 0.97; }
.pic-box { border-radius: 10px; overflow: hidden; }
.pic-bg { background: var(--surface); }
.pic-icon { font-size: 40px; color: var(--muted); }
.dur-place { padding: 6px; }
.dur { padding: 2px 6px; border-radius: 6px; background: #000000b3; }
.dur-text { font-size: 12px; font-weight: 600; color: #ffffff; }
.card-title { font-size: 13px; padding: 0px 2px; }

.empty { flex-grow: 1; padding: 24px; }
.empty-icon { font-size: 64px; color: var(--muted); }
.empty-text { color: var(--muted); text-align: center; padding: 12px 0px 0px 0px; }
.error { margin: 0px 12px 8px 12px; padding: 10px 14px; border-radius: 12px; background: var(--danger); }
.error-text { color: #ffffff; }

/* плеер */
.player-bg { background: #000000; }
.player-center { flex-grow: 1; }
.player-error-text { color: #ffffff; text-align: center; padding: 0px 24px; }
.player-title { font-size: 16px; font-weight: 600; color: #ffffff; flex-shrink: 1; }
