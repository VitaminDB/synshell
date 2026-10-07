/* syn-audio-player — «Музыка». Палитра (--bg, --fg, --accent, …) — из [appearance]. */

.grow { flex-grow: 1; }
.root { background: var(--bg); }
Text { color: var(--fg); font-size: 14px; }

.header { padding: 12px 12px 8px 16px; }
.title { font-size: 22px; font-weight: 600; }
.now { font-size: 15px; font-weight: 600; color: var(--muted); }
.ib { width: 44px; height: 44px; border-radius: 22px; padding: 10px; transition: background-color 120ms ease-out, scale 160ms spring(500, 28); }
.ib:hover { background-color: var(--hover); }
.ib:active { background-color: var(--pressed); scale: 0.92; }
.ib-icon { font-size: 24px; color: var(--fg); }

/* список */
.list { padding: 0px 8px 12px 8px; }
.row { padding: 8px 10px; border-radius: 12px; transition: background-color 120ms ease-out, scale 160ms spring(500, 28); }
.row:hover { background-color: var(--hover); }
.row:active { background-color: var(--pressed); scale: 0.98; }
.row-active { background-color: var(--surface); }
.row-icon-box { width: 40px; height: 40px; border-radius: 10px; background: var(--surface); padding: 8px; }
.row-icon { font-size: 24px; color: var(--muted); }
.row-icon.active { color: var(--accent); }
.row-title { font-size: 15px; }
.row-title.active { color: var(--accent); font-weight: 600; }
.row-sub { font-size: 12px; color: var(--muted); }
.row-dur { font-size: 12px; color: var(--muted); }

.empty { flex-grow: 1; padding: 24px; }
.empty-icon { font-size: 64px; color: var(--muted); }
.empty-text { color: var(--muted); text-align: center; padding: 12px 0px 0px 0px; }
.error { margin: 0px 12px 8px 12px; padding: 10px 14px; border-radius: 12px; background: var(--danger); }
.error-text { color: #ffffff; }

/* «сейчас играет» внизу списка */
.bar-wrap { background: var(--surface); }
.bar-progress { height: 3px; }
.bar { padding: 8px 8px 8px 10px; }
.bar-cover { width: 48px; height: 48px; border-radius: 10px; overflow: hidden; }
.bar-cover-icon { font-size: 28px; color: var(--muted); }
.bar-title { font-size: 14px; font-weight: 600; }

/* обложка */
.cover-bg { background: var(--surface); }
.cover-img { }

/* плеер */
.player-body { flex-grow: 1; padding: 8px 28px; }
.big-cover-box { width: 300px; border-radius: 20px; overflow: hidden; }
.big-cover { border-radius: 20px; overflow: hidden; }
.big-cover-icon { font-size: 120px; color: var(--muted); }
.big-title { font-size: 20px; font-weight: 600; text-align: center; }
.big-sub { font-size: 14px; color: var(--muted); text-align: center; }
.seek-box { padding: 0px 24px; }
.time { font-size: 12px; color: var(--muted); }
.controls { padding: 12px 16px 28px 16px; }
.ctl { width: 52px; height: 52px; border-radius: 26px; padding: 12px; transition: background-color 120ms ease-out, scale 160ms spring(500, 28); }
.ctl:hover { background-color: var(--hover); }
.ctl:active { background-color: var(--pressed); scale: 0.9; }
.ctl-icon { font-size: 28px; color: var(--fg); }
.ctl-icon.dim { color: var(--muted); }
.ctl-icon.active { color: var(--accent); }
.ctl-main { width: 72px; height: 72px; border-radius: 36px; padding: 16px; background: var(--accent); transition: scale 160ms spring(500, 28); }
.ctl-main:active { scale: 0.92; }
.ctl-main-icon { font-size: 40px; color: var(--accent-fg); }
