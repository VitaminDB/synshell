/* synsms — «Сообщения». Переменные палитры (--bg, --fg, --accent…) — из [appearance]. */

.root { background: var(--bg); flex-grow: 1; }
Text { color: var(--fg); font-size: 14px; }
.grow { flex-grow: 1; }
.muted { color: var(--muted); font-size: 13px; }
.empty { padding: 120px 16px 16px 16px; width: 100%; }
.empty-icon { font-size: 48px; color: var(--muted); }

.side { width: 340px; border-right-width: 1px; border-color: var(--border); background: var(--surface); }
.main { background: var(--bg); }

.bar { padding: 10px 12px; min-height: 56px; }
.title { font-size: 22px; font-weight: 600; padding: 6px 6px; }
.bar-title { font-size: 17px; font-weight: 600; }
.icon-btn { width: 40px; height: 40px; border-radius: 20px; padding: 8px; transition: background-color 120ms ease-out; }
.icon-btn:hover { background-color: var(--hover); }
.icon-btn:active { background-color: var(--pressed); }
.btn-icon { font-size: 24px; color: var(--fg); }

.note { margin: 0px 12px 8px 12px; padding: 8px 12px; border-radius: var(--radius-sm); background: var(--surface-alt); }
.note-text { font-size: 12px; color: var(--muted); }

.list { padding: 0px 8px 96px 8px; }
.conv { padding: 10px 10px; border-radius: var(--radius); background-color: #00000000; transition: background-color 120ms ease-out; }
.conv:hover { background-color: var(--hover); }
.conv:active { background-color: var(--pressed); }
.conv-on { background-color: var(--accent-soft); }
.conv-name { font-size: 15px; }
.conv-unread { font-weight: 600; }
.conv-last { font-size: 13px; color: var(--muted); }
.conv-time { font-size: 11px; color: var(--muted); }
.badge { min-width: 20px; height: 20px; padding: 0px 6px; border-radius: 10px; background: var(--accent); align-items: center; justify-content: center; }
.badge-text { font-size: 11px; font-weight: 600; color: #ffffff; }

.avatar { width: 40px; height: 40px; border-radius: 20px; background: var(--accent-soft); }
.avatar-text { font-size: 17px; font-weight: 600; color: var(--accent); }
.avatar-icon { font-size: 22px; color: var(--accent); }

.fab-place { padding: 0px 20px 24px 0px; }
.fab { width: 56px; height: 56px; border-radius: 18px; padding: 16px; background: var(--accent); box-shadow: 0 6px 18px var(--shadow); transition: scale 200ms spring(420, 26); }
.fab:active { scale: 0.94; }
.fab-icon { font-size: 24px; color: #ffffff; }

.thread { padding: 8px 12px 12px 12px; }
.day-row { padding: 10px 0px 4px 0px; }
.day { font-size: 11px; color: var(--muted); }
.bubble { padding: 8px 12px; border-radius: 18px; max-width: 78%; }
.bubble-in { background: var(--bubble-in); border-bottom-left-radius: 6px; }
.bubble-out { background: var(--accent); border-bottom-right-radius: 6px; }
.bubble-failed { background: var(--danger); }
.bubble-text { font-size: 15px; }
.bubble-text-out { color: #ffffff; }
.bubble-meta { font-size: 10px; color: var(--muted); }
.bubble-meta-out { color: #ffffffb0; }

.composer { padding: 8px 10px 10px 12px; border-top-width: 1px; border-color: var(--border); background: var(--surface); }
.input { background: var(--input-bg); border-radius: 22px; padding: 10px 14px; min-height: 44px; }
.send { width: 44px; height: 44px; border-radius: 22px; padding: 10px; background: var(--surface-alt); }
.send-on { background: var(--accent); }
.send-icon { font-size: 22px; color: #ffffff; }
.to-row { padding: 4px 12px 8px 12px; }

.btn { padding: 8px 14px; border-radius: 999px; background: var(--surface-alt); }
.btn-text { font-size: 13px; font-weight: 600; }
.btn-danger { background: var(--danger); }
.btn-danger-text { color: #ffffff; }

.toast-place { padding: 0px 16px 96px 16px; }
.toast { padding: 10px 16px; border-radius: 999px; background: var(--fg); box-shadow: 0 6px 20px var(--shadow); }
.toast-text { font-size: 13px; color: var(--bg); }
