/* syn-audio-recorder — «Диктофон». Палитра (--bg, --fg, --accent, …) — из [appearance], свои переменные — в main.rs:
   --rec (красный записи), --rec-2, --rec-soft, --rec-ring, --rec-glow, --card, --titlebar, --sidebar, --content. */

.grow { flex-grow: 1; }
.ic { icon-size: 20px; icon-color: var(--muted); }
Text { color: var(--fg); font-size: 14px; }
.section { font-size: 12px; font-weight: 600; color: var(--muted); padding: 6px 2px 0px 2px; }

/* ── Окно ─────────────────────────────────────────────────────────── */

.window { background: var(--content); color: var(--fg); }
.window.phone { background: var(--bg); }
.window-frame { border-radius: var(--window-radius); border: 1px solid var(--border); }
.window-frame.flat { border-radius: 0px; border-width: 0px; }
.window-frame:window-maximized, .window-frame:window-fullscreen { border-radius: 0px; border-width: 0px; }
.body { background: var(--content); }
.window.phone .body { background: var(--bg); }

.titlebar { height: 46px; background: var(--titlebar); }
.tb-left { height: 46px; width: 340px; padding: 0px 10px 0px 12px; align-items: center; }
.drag-wrap { height: 46px; }
.drag-space { height: 46px; flex-grow: 1; min-width: 40px; align-items: center; justify-content: center; }
.tb-tools { height: 46px; padding: 0px 4px; align-items: center; }
.window-controls { height: 46px; padding: 0px 4px; align-items: center; }
SystemWindowControls { color: var(--fg); background-color: var(--hover); }

.brand { width: 28px; height: 28px; border-radius: 9px; background: linear-gradient(135deg, var(--rec), var(--rec-2)); justify-content: center; align-items: center; }
.brand-ic { icon-color: #ffffff; icon-size: 18px; }
.brand-title { font-size: 14px; font-weight: 600; color: var(--fg); }

.appbar { height: 60px; padding: 6px 8px 6px 8px; background: var(--bg); }
.appbar-mark { padding: 0px 6px 0px 8px; }
.appbar-drag { height: 48px; align-items: center; padding: 0px 6px; }
.appbar-title { font-size: 19px; font-weight: 600; color: var(--fg); }

/* значок «идёт запись» */
.rec-badge { padding: 5px 11px 5px 9px; border-radius: 999px; background-color: var(--rec-soft); margin: 0px 4px; }
.rec-dot { width: 8px; height: 8px; border-radius: 4px; background-color: var(--rec); }
.rec-badge.paused .rec-dot { background-color: var(--muted); }
.rec-badge-text { font-size: 12px; font-weight: 600; color: var(--rec); }
.rec-badge.paused .rec-badge-text { color: var(--muted); }

.list-chip { padding: 8px 12px 8px 10px; border-radius: 999px; background-color: var(--surface); margin: 0px 2px; transition: background-color 120ms ease-out; }
.list-chip:hover { background-color: var(--hover); }
.list-chip-ic { icon-size: 20px; icon-color: var(--fg); }
.list-chip-text { font-size: 14px; font-weight: 600; }

/* ── Кнопки ───────────────────────────────────────────────────────── */

.btn { padding: 9px 14px; border-radius: 12px; background-color: var(--surface); color: var(--fg); font-size: 13px; icon-color: var(--fg); transition: background-color 120ms ease-out, scale 160ms spring(500, 28); }
.btn:hover { background-color: var(--hover); }
.btn:pressed { background-color: var(--pressed); }
.btn.flat { background-color: #00000000; icon-color: var(--muted); }
.btn.flat:hover { background-color: var(--hover); }
.btn.round { padding: 8px; border-radius: 999px; }
.btn.primary { background-color: var(--rec); color: #ffffff; icon-color: #ffffff; font-weight: 600; }
.btn.primary:hover { background-color: var(--rec-2); }
.btn.tb-btn { padding: 9px; icon-size: 21px; icon-color: var(--muted); }
.btn.tb-btn:hover { icon-color: var(--fg); }
.small-btn { font-size: 12px; padding: 6px 10px; }

/* ── Экран записи ─────────────────────────────────────────────────── */

.rec-max { max-width: 760px; }
.rec-page { flex-grow: 1; padding: 18px 28px 14px 28px; }
.rec-page.phone { padding: 6px 16px 26px 16px; }
.min-gap { min-height: 14px; }
.gap-24 { height: 16px; }
.half { flex-basis: 0px; }

.src-item { padding: 12px 14px; border-radius: 18px; background-color: var(--card); border: 1px solid var(--border); transition: background-color 120ms ease-out; }
.src-item:hover { background-color: var(--hover); }
.src-item.locked { opacity: 0.6; }
.src-badge { width: 40px; height: 40px; border-radius: 13px; background-color: var(--surface-alt); justify-content: center; align-items: center; }
.src-badge.on { background-color: var(--accent-dim); }
.src-badge.on .src-ic { icon-color: var(--accent); }
.src-ic { icon-size: 22px; icon-color: var(--fg); }
.src-chev { icon-size: 22px; }
.src-cap { font-size: 12px; color: var(--muted); }
.src-cap.warn { color: var(--warning); }
.src-value { font-size: 15px; font-weight: 600; color: var(--fg); }

.status { padding: 5px 12px; border-radius: 999px; background-color: var(--surface); }
.status-dot { width: 8px; height: 8px; border-radius: 4px; background-color: var(--muted); }
.status-text { font-size: 13px; font-weight: 600; color: var(--muted); }
.status.rec { background-color: var(--rec-soft); }
.status.rec .status-dot { background-color: var(--rec); box-shadow: 0 0 8px var(--rec-glow); }
.status.rec .status-text { color: var(--rec); }
.status.paused .status-dot { background-color: var(--warning); }
.status.paused .status-text { color: var(--fg); }
.status.err .status-dot { background-color: var(--danger); }
.status.err .status-text { color: var(--danger); }

.timer { font-size: 58px; font-weight: 300; color: var(--muted); letter-spacing: 1px; }
.timer.on { color: var(--fg); }
.timer.frac { font-size: 30px; padding: 0px 0px 8px 2px; color: var(--muted); }
.window.phone .timer { font-size: 54px; }
.window.phone .timer.frac { font-size: 28px; padding: 0px 0px 8px 2px; }
.fmt { font-size: 12px; color: var(--muted); }

.vis-card { padding: 16px 16px 14px 16px; border-radius: 26px; background-color: var(--card); border: 1px solid var(--border); box-shadow: 0 10px 30px var(--shadow); }
.vis { height: 150px; color: var(--fg); accent-color: var(--rec); }
.vis.phone { height: 168px; }
.meter { height: 6px; color: var(--fg); }
.clip { padding: 2px 8px; border-radius: 6px; background-color: #00000000; }
.clip-text { font-size: 11px; font-weight: 600; color: #00000000; }
.clip.on { background-color: var(--danger); }
.clip.on .clip-text { color: #ffffff; }

/* кнопки записи */
.side-wrap { width: 76px; }
.side-wrap.off { opacity: 0.35; }
.btn.side { width: 56px; height: 56px; padding: 15px; border-radius: 28px; background-color: var(--surface); icon-size: 26px; icon-color: var(--fg); }
.btn.side:hover { background-color: var(--hover); }
.side-label { font-size: 12px; color: var(--muted); }

.rec-ring { width: 92px; height: 92px; border-radius: 46px; padding: 8px; border: 3px solid var(--rec-ring); justify-content: center; align-items: center; transition: border-color 200ms ease-out, scale 160ms spring(500, 28); }
.rec-ring:hover { border-color: var(--rec); }
.rec-ring:active { scale: 0.93; }
.rec-ring.live { border-color: var(--rec); box-shadow: 0 0 26px var(--rec-glow); }
.rec-core { width: 70px; height: 70px; border-radius: 35px; background: linear-gradient(135deg, var(--rec-2), var(--rec)); box-shadow: 0 6px 18px var(--rec-glow); transition: border-radius 220ms ease-out, width 220ms ease-out, height 220ms ease-out; }
.rec-core.stop { width: 32px; height: 32px; border-radius: 9px; }
.rec-core.busy { opacity: 0.6; }

.keys-hint { font-size: 11px; color: var(--muted); text-align: center; padding: 10px 0px 0px 0px; }

/* ── Записи ───────────────────────────────────────────────────────── */

.sidebar { width: 360px; background: var(--sidebar); border-right: 1px solid var(--border); }
.list-head { padding: 16px 12px 8px 18px; }
.list-title { font-size: 18px; font-weight: 700; color: var(--fg); }
.list-page { flex-grow: 1; }
.rows { padding: 2px 10px 12px 10px; }

.row { padding: 10px 12px 10px 10px; border-radius: 16px; transition: background-color 120ms ease-out, scale 160ms spring(500, 28); }
.row:hover { background-color: var(--hover); }
.row:active { scale: 0.985; }
.row.on { background-color: var(--card); border: 1px solid var(--border); box-shadow: 0 4px 14px var(--shadow); }
.row-play { width: 40px; height: 40px; border-radius: 20px; background-color: var(--surface); justify-content: center; align-items: center; }
.row-play.on { background: linear-gradient(135deg, var(--rec-2), var(--rec)); }
.row-play-ic { icon-size: 22px; icon-color: var(--fg); }
.row-play.on .row-play-ic { icon-color: #ffffff; }
.row-title { font-size: 14px; font-weight: 600; color: var(--fg); }
.row-title.on { color: var(--fg); }
.row-meta { font-size: 12px; color: var(--muted); }
.thumb { width: 66px; height: 28px; color: var(--muted); accent-color: var(--rec); }
.thumb.on { color: var(--rec); }

.empty { flex-grow: 1; padding: 40px 24px; }
.empty-badge { width: 84px; height: 84px; border-radius: 42px; background-color: var(--rec-soft); justify-content: center; align-items: center; }
.empty-ic { icon-size: 42px; icon-color: var(--rec); }
.empty-text { color: var(--muted); text-align: center; font-size: 14px; }

/* плеер */
.player-wrap { padding: 6px 10px 10px 10px; }
.window.phone .player-wrap { padding: 6px 12px 16px 12px; }
.player { padding: 12px 14px 12px 16px; border-radius: 22px; background-color: var(--card); border: 1px solid var(--border); box-shadow: 0 -6px 24px var(--shadow); }
.window.phone .player { padding: 14px 16px 16px 18px; }
.pl-title { font-size: 15px; font-weight: 600; color: var(--fg); }
.btn.pl-act { padding: 7px; icon-size: 20px; }
.btn.pl-act.danger:hover { icon-color: var(--danger); }
.seek-wave { height: 56px; color: var(--fg); accent-color: var(--rec); }
.pl-time { font-size: 12px; color: var(--muted); }
.btn.pl-skip { padding: 10px; icon-size: 26px; icon-color: var(--fg); }
.btn.pl-main { width: 56px; height: 56px; padding: 14px; border-radius: 28px; background: linear-gradient(135deg, var(--rec-2), var(--rec)); icon-size: 28px; icon-color: #ffffff; box-shadow: 0 6px 16px var(--rec-glow); }
.btn.pl-main:hover { background: linear-gradient(135deg, var(--rec), var(--rec-2)); }

/* ── Листы и подсказки ────────────────────────────────────────────── */

.scrim { width: 100%; height: 100%; background-color: var(--scrim); }
.sheet-wrap { width: 100%; height: 100%; padding: 24px; }
.sheet-wrap.bottom { padding: 40px 0px 0px 0px; }
.sheet { padding: 18px 18px 20px 20px; background-color: var(--card); border: 1px solid var(--border); box-shadow: 0 16px 40px var(--shadow); }
.sheet.card { width: 460px; max-height: 620px; border-radius: 24px; }
.sheet.bottom { border-radius: 26px 26px 0px 0px; padding: 10px 16px 28px 18px; max-height: 760px; }
.grabber { width: 40px; height: 4px; border-radius: 2px; background-color: var(--border); }
.sheet-title { font-size: 18px; font-weight: 700; color: var(--fg); }

.opt { padding: 12px 14px; border-radius: 16px; background-color: var(--surface); border: 1px solid #00000000; transition: background-color 120ms ease-out; }
.opt:hover { background-color: var(--hover); }
.opt.on { background-color: var(--rec-soft); border: 1px solid var(--rec-ring); }
.opt-ic { icon-size: 22px; }
.opt-ic.on { icon-color: var(--rec); }
.opt-radio { icon-size: 22px; }
.opt-radio.on { icon-color: var(--rec); }
.opt-title { font-size: 15px; color: var(--fg); }
.opt-title.on { font-weight: 600; }
.opt-sub { font-size: 12px; color: var(--muted); }
.opt-sub.pad { padding: 8px 4px; }
SegmentedButton.seg { height: 40px; background: var(--surface); color: var(--fg); accent-color: var(--rec); border-width: 1px; border-color: var(--border); border-radius: 12px; font-size: 13px; }

TextField.field { background-color: var(--surface); border: 1px solid var(--border); border-radius: 12px; padding: 10px 12px; font-size: 14px; color: var(--fg); }
TextField.field:focus { border-color: var(--accent); }

.toast-wrap { width: 100%; height: 100%; padding: 0px 16px 30px 16px; }
.toast { padding: 10px 12px 10px 18px; border-radius: 16px; background-color: var(--card); border: 1px solid var(--border); box-shadow: 0 8px 24px var(--shadow); max-width: 480px; min-height: 44px; }
.toast.error { border-color: var(--danger); }
.toast-text { font-size: 13px; color: var(--fg); }
.toast-warn { icon-color: var(--danger); }
.btn.toast-btn { padding: 7px 12px; background-color: #00000000; color: var(--accent); font-weight: 600; }
.btn.toast-btn:hover { background-color: var(--hover); }
