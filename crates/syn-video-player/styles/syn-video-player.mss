/* syn-video-player — «Видео». Палитра (--bg, --fg, --accent, …) — из [appearance]. */

.grow { flex-grow: 1; }
.root { background: var(--bg); }

/* своё окно: рамка, перетаскивание за шапку, кнопки окна */
.window-frame { border-radius: var(--window-radius); border: 1px solid var(--border); background-color: var(--bg); }
.window-frame.flat { border-radius: 0px; border-width: 0px; }
.window-frame:window-maximized, .window-frame:window-fullscreen { border-radius: 0px; border-width: 0px; }
.drag { min-width: 40px; }
.drag-space { flex-grow: 1; min-height: 44px; align-items: center; }
.window-controls { padding: 0px 0px 0px 6px; align-items: center; }
SystemWindowControls { color: var(--fg); background-color: var(--hover); }
.vp-wc SystemWindowControls { color: #ffffff; background-color: #ffffff29; }
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

/* Монтаж (палитра тёмная — экран чёрный, как у плеера). */
.ed-top { padding: 8px 10px 6px 6px; background: #000000; }
.ed-preview { flex-grow: 1; flex-shrink: 1; min-height: 0px; overflow: hidden; }
.ed-panel { padding: 10px 14px 16px 14px; background: #121212; }
.ed-track { height: 46px; }
.ed-seg { flex-grow: 1; border-radius: 6px; border: 2px solid rgba(255, 255, 255, 0.0); overflow: hidden; transition: border-color 100ms ease; }
.ed-src0 { background: #3d6fd8; }
.ed-src1 { background: #2e9c6a; }
.ed-src2 { background: #b8792a; }
.ed-src3 { background: #9a4fc2; }
.ed-seg-off { background: rgba(255, 255, 255, 0.10); }
.ed-seg-on { border-color: #ffffff; }
.ed-head { width: 2px; background: #ffffff; }
.ed-info { font-size: 13px; color: #ffffffe6; }
.ed-hint { font-size: 12px; color: #ffffff99; flex-shrink: 1; }
.ed-tool { width: 68px; padding: 6px 2px; border-radius: 12px; transition: background-color 120ms ease-out, scale 160ms spring(500, 28); }
.ed-tool:hover { background-color: rgba(255, 255, 255, 0.08); }
.ed-tool:active { background-color: rgba(255, 255, 255, 0.16); scale: 0.94; }
.ed-tool-icon { font-size: 24px; color: #ffffff; }
.ed-tool-text { font-size: 11px; color: #ffffffcc; }
.ed-tool-off { opacity: 0.35; }
.ed-ib { width: 40px; height: 40px; border-radius: 20px; padding: 8px; }
.ed-ib:hover { background-color: rgba(255, 255, 255, 0.10); }
.ed-ib-icon { font-size: 24px; color: #ffffff; }
.ed-btn { padding: 8px 16px; border-radius: 999px; background: var(--surface); transition: scale 160ms spring(500, 28); }
.ed-btn:active { scale: 0.95; }
.ed-btn-primary { background: var(--accent); }
.ed-btn-icon { font-size: 18px; color: var(--fg); }
.ed-btn-text { font-size: 14px; font-weight: 600; color: var(--fg); }
.ed-btn-primary .ed-btn-icon, .ed-btn-primary .ed-btn-text { color: #ffffff; }
.ed-dialog { width: 420px; max-width: 92%; padding: 20px; border-radius: 18px; background: var(--bg); box-shadow: 0px 8px 32px #00000080; }
.ed-file { font-size: 13px; color: var(--fg); }
.ed-progress { height: 6px; }
.ed-add-hint { font-size: 12px; color: var(--muted); padding: 0px 20px 8px 20px; }
.ed-add-row { padding: 6px; border-radius: 12px; }
.ed-add-row:hover { background-color: var(--hover); }
.ed-add-thumb { width: 96px; height: 54px; border-radius: 8px; overflow: hidden; }
.ed-dur-place { padding: 3px; }
