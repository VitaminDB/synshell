/* syn-video-editor — своё поверх стилей hexspire (переменные — из палитры synshell, ui/mod.rs). */

.grow { flex-grow: 1; }
.logo-icon { icon-size: 18px; icon-color: var(--primary); }
.window-controls { height: 40px; align-items: center; padding: 0 6px 0 0; }

.tool-pill { height: 30px; padding: 0 12px; border-radius: var(--radius-pill); background-color: var(--primary-soft); cursor: pointer; }
.tool-pill:hover { background-color: var(--primary); }
.tool-pill-icon { icon-size: 16px; icon-color: var(--primary); }
.tool-pill-text { font-size: 12px; font-weight: 600; color: var(--text); }
.view-select { width: 150px; }

.panel-empty { color: var(--text-subtle); font-size: 12px; text-align: center; padding: 16px; }
.panel-hint { color: var(--text-muted); font-size: 11px; padding: 4px 6px 6px 6px; }
.panel-list { padding: 6px; }
.list-row { padding: 6px 8px; border-radius: 8px; cursor: pointer; }
.list-row:hover { background-color: var(--surface-hover); }
.row-icon { icon-size: 18px; icon-color: var(--text-muted); }
.row-title { font-size: 12px; color: var(--text); }
.row-sub { font-size: 11px; color: var(--text-subtle); }

.section-title { font-size: 11px; font-weight: 600; color: var(--text-subtle); padding: 10px 8px 4px 8px; }
.inspector { padding: 4px 6px; }
.kv-row { padding: 4px 8px; min-height: 24px; }
.kv-key { font-size: 12px; color: var(--text-muted); width: 120px; }
.kv-value { font-size: 12px; color: var(--text); font-family: monospace; }

/* ── просмотр ── */
.viewer { background-color: #000000; flex-grow: 1; }
.viewer-title { margin: 0 0 32px 0; padding: 6px 14px; border-radius: 8px; background-color: #00000099; }
.viewer-title-text { color: #FFFFFF; font-size: 26px; font-weight: 700; text-align: center; }

/* ── таймлайн ── */
.timeline { background-color: var(--bg-hex); flex-grow: 1; }
.tl-heads { background-color: var(--bg-panel); border-right-width: 1px; border-color: var(--border-soft); }
.tl-corner { height: 24px; padding: 0 8px; justify-content: center; border-bottom-width: 1px; border-color: var(--border-soft); }
.tl-time { font-family: monospace; font-size: 11px; color: var(--primary); }
.tl-head { padding: 0 10px; justify-content: center; border-bottom-width: 1px; border-color: var(--border-soft); }
.tl-head-v { height: 52px; }
.tl-head-a { height: 38px; }
.tl-head-text { font-size: 12px; font-weight: 600; color: var(--text-muted); }
.tl-ruler { background-color: var(--bg-panel); border-bottom-width: 1px; border-color: var(--border-soft); }
.tl-tick { background-color: var(--text-subtle); }
.tl-tick-text { font-size: 10px; color: var(--text-subtle); font-family: monospace; }
.tl-row { border-bottom-width: 1px; border-color: var(--border-soft); }
.tl-playhead { background-color: var(--error); }
.tl-playhead-cap { background-color: var(--error); border-radius: 3px; }

.clip { border-radius: 6px; padding: 4px 8px; border-width: 1px; border-color: #FFFFFF22; cursor: pointer; }
.clip-video { background-color: var(--clip-video); }
.clip-audio { background-color: var(--clip-audio); }
.clip-title { background-color: var(--clip-title); }
.clip-color { background-color: var(--clip-color); }
.clip:hover { border-color: #FFFFFF66; }
.clip-selected { border-width: 2px; border-color: var(--warning); }
.clip-text { font-size: 11px; color: #FFFFFF; }

/* ── ИИ ── */
.ai-panel { flex-grow: 1; }
.ai-log { padding: 8px; }
.ai-user { padding: 8px 10px; border-radius: 10px; background-color: var(--primary-soft); }
.ai-user-text { color: var(--text); font-size: 12px; }
.ai-text { padding: 2px 4px; }
.ai-text-text { color: var(--text); font-size: 12px; }
.ai-tool { padding: 2px 4px; }
.ai-tool-text { color: var(--warning); font-size: 11px; font-family: monospace; }
.ai-result { padding: 0 4px 2px 16px; }
.ai-result-text { color: var(--text-subtle); font-size: 11px; font-family: monospace; }
.ai-error { padding: 6px 8px; border-radius: 8px; background-color: var(--warning-soft); }
.ai-error-text { color: var(--error); font-size: 12px; }
.ai-input { padding: 8px; border-top-width: 1px; border-color: var(--border-soft); }
.ai-field { flex-grow: 1; }

.toast { padding: 10px 14px; border-radius: 10px; background-color: var(--bg-panel-2); border-width: 1px; border-color: var(--border); box-shadow: 0 6px 16px rgba(0, 0, 0, 0.35); max-width: 520px; }
.toast-text { color: var(--text); font-size: 12px; }
.toast-layer { padding: 0 34px 46px 0; flex-grow: 1; }
.panel-head { padding: 6px 8px; border-bottom-width: 1px; border-color: var(--border-soft); }
.head-btn { height: 26px; font-size: 12px; }
.media-empty { padding: 28px 12px; }
.media-empty-icon { icon-size: 36px; icon-color: var(--text-subtle); }

/* ── чипы заголовка: сборка (как hexspire), Donate и GitHub (как Synthos) ── */
.build-chip { height: 20px; padding: 0px 8px 0px 7px; border-radius: 10px; border-width: 1px; border-color: var(--border-soft); background-color: var(--primary-soft); }
.build-chip-dot { width: 6px; height: 6px; border-radius: 3px; background-color: var(--primary); }
.build-chip-text { font-family: monospace; font-size: 11px; font-weight: 600; color: var(--text); }
.titlebar-chip { height: 20px; padding: 0 8px 0 7px; border-radius: var(--radius-pill); background-color: transparent; cursor: pointer; transition: background-color 120ms ease-out; }
.titlebar-chip:hover { background-color: var(--surface-hover); }
.titlebar-chip.titlebar-chip-donate:hover { background-color: rgba(229, 72, 122, 0.12); }
.titlebar-chip-text { font-size: 9px; font-weight: 700; letter-spacing: 0.6px; text-transform: uppercase; color: var(--text-muted); }
.titlebar-chip-icon { color: #E5487A; icon-size: 11px; }
.titlebar-chip-logo { width: 11px; height: 11px; color-tint: var(--text-muted); }
.titlebar-chip--hover .titlebar-chip-text { color: var(--text); }
.titlebar-chip--hover .titlebar-chip-logo { color-tint: var(--text); }
.titlebar-chip-donate.titlebar-chip--hover .titlebar-chip-text { color: #E5487A; }
