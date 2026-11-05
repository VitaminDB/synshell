/* synphone — «Телефон». Переменные палитры (--bg, --fg, --accent…) — из [appearance]. */

.root { background: var(--bg); flex-grow: 1; }
Text { color: var(--fg); font-size: 14px; }
.grow { flex-grow: 1; }
.muted { color: var(--muted); font-size: 13px; }
.empty { padding: 120px 16px 16px 16px; width: 100%; }
.empty-icon { font-size: 48px; color: var(--muted); }
.title { font-size: 22px; font-weight: 600; padding: 14px 18px 8px 18px; }

.net { padding: 8px 12px 0px 12px; }
.net-ok { font-size: 12px; color: var(--muted); }
.net-bad { font-size: 12px; color: var(--danger); }

.dial { padding: 0px 16px 8px 16px; }
.display-row { padding: 0px 8px 18px 8px; }
.display { font-size: 34px; max-width: 300px; }
.display-side { width: 44px; height: 44px; padding: 10px; }
.bs-icon { font-size: 24px; color: var(--muted); }
.key { width: 76px; height: 76px; border-radius: 38px; background: var(--surface-alt); transition: background-color 100ms ease-out, scale 160ms spring(500, 28); }
.key:hover { background-color: var(--hover); }
.key:active { background-color: var(--pressed); scale: 0.94; }
.key-digit { font-size: 28px; }
.key-letters { font-size: 9px; color: var(--muted); }
.call-row { padding: 18px 0px 10px 0px; }

.round { width: 68px; height: 68px; border-radius: 34px; padding: 20px; transition: scale 160ms spring(500, 28); }
.round:active { scale: 0.92; }
.round-icon { font-size: 28px; color: #ffffff; }
.round-green { background: #2e9d48; }
.round-red { background: #e5383b; }
.round-soft { background: var(--surface-alt); }
.round-soft .round-icon { color: var(--fg); }
.round-soft-on { background: var(--accent); }
.round-soft-on .round-icon { color: #ffffff; }
.round-label { font-size: 12px; color: var(--muted); }

.list { padding: 0px 8px 16px 8px; }
.log-row { padding: 10px 10px; border-radius: var(--radius); background-color: #00000000; transition: background-color 120ms ease-out; }
.log-row:hover { background-color: var(--hover); }
.log-row:active { background-color: var(--pressed); }
.log-icon { font-size: 22px; color: var(--muted); }
.log-missed { color: var(--danger); }
.log-name { font-size: 15px; }
.log-name-missed { color: var(--danger); }
.log-sub { font-size: 12px; color: var(--muted); }
.row-btn { width: 40px; height: 40px; border-radius: 20px; padding: 9px; }
.row-btn:active { background-color: var(--pressed); }
.row-btn-icon { font-size: 22px; color: var(--accent); }

.in-call { padding: 64px 24px 48px 24px; }
.call-info { width: 100%; }
.big-avatar { width: 112px; height: 112px; border-radius: 56px; background: var(--accent-soft); }
.big-avatar-icon { font-size: 56px; color: var(--accent); }
.call-number { font-size: 28px; font-weight: 600; }
.call-state { font-size: 16px; color: var(--muted); }
.call-buttons { width: 100%; padding: 8px 0px; }

.navbar { padding: 6px 0px 10px 0px; border-top-width: 1px; border-color: var(--border); background: var(--surface); }
.nb-item { padding: 4px 18px; }
.nb-pill { padding: 4px 18px; border-radius: 16px; }
.nb-pill-on { background: var(--accent-soft); }
.nb-icon { font-size: 22px; color: var(--fg); }
.nb-label { font-size: 11px; }
.nb-dot { width: 6px; height: 6px; border-radius: 3px; background: var(--danger); }

.toast-place { padding: 0px 16px 96px 16px; }
.toast { padding: 10px 16px; border-radius: 999px; background: var(--fg); box-shadow: 0 6px 20px var(--shadow); }
.toast-text { font-size: 13px; color: var(--bg); }
