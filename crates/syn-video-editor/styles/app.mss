/* syn-video-editor — своё поверх стилей hexspire (переменные — из палитры synshell, ui/mod.rs). */

.grow { flex-grow: 1; }
.logo-icon { icon-size: 18px; icon-color: var(--primary); }
.window-controls { height: 40px; align-items: center; padding: 0 6px 0 0; }

.tool-pill { height: 30px; padding: 0 12px; border-radius: var(--radius-pill); background-color: var(--primary-soft); cursor: pointer; }
.tool-pill:hover { background-color: var(--primary); }
.tool-pill-icon { icon-size: 16px; icon-color: var(--primary); }
.tool-pill-text { font-size: 12px; font-weight: 600; color: var(--text); }
.view-select { width: 150px; --popup-min-width: 230px; }

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
.viewer { background-color: #1c1e26; flex-grow: 1; }
.viewer-frame-box { border-width: 1px; border-color: #FFFFFF52; box-shadow: 0 4px 20px rgba(0, 0, 0, 0.55); }
.viewer-zoom-select { width: 106px; height: 24px; font-size: 11px; --popup-min-width: 112px; }
.transport-res { font-family: monospace; font-size: 11px; color: var(--text-subtle); }
.viewer-title { margin: 0 0 32px 0; padding: 6px 14px; border-radius: 8px; background-color: #00000099; }
.viewer-title-text { color: #FFFFFF; font-size: 26px; font-weight: 700; text-align: center; }

/* ── таймлайн ── */
.timeline { background-color: var(--bg-hex); flex-grow: 1; }
.tl-heads { background-color: var(--bg-panel); border-right-width: 1px; border-color: var(--border-soft); }
.tl-corner { height: 24px; padding: 0 8px; justify-content: center; border-bottom-width: 1px; border-color: var(--border-soft); }
.tl-time { font-family: monospace; font-size: 11px; color: var(--primary); }
.tl-head { padding: 0 4px 0 10px; justify-content: center; border-bottom-width: 1px; border-color: var(--border-soft); }


.tl-head-text { font-size: 12px; font-weight: 600; color: var(--text-muted); }
.tl-ruler { background-color: var(--bg-panel); border-bottom-width: 1px; border-color: var(--border-soft); }
.tl-tick { background-color: var(--text-subtle); }
.tl-tick-text { font-size: 10px; color: var(--text-subtle); font-family: monospace; }
.tl-row { border-bottom-width: 1px; border-color: var(--border-soft); }
.tl-playhead { background-color: var(--error); }
.tl-playhead-cap { background-color: var(--error); border-radius: 3px; }

.clip { border-radius: 6px; padding: 0px; border-width: 1px; border-color: #FFFFFF22; cursor: pointer; }
.clip-video { background-color: var(--clip-video); }
.clip-audio { background-color: var(--clip-audio); }
.clip-title { background-color: var(--clip-title); }
.clip-shape { background-color: var(--clip-shape); }
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

/* ── плитки медиатеки ── */
.tile { padding: 6px; border-radius: 10px; cursor: pointer; }
.tile:hover { background-color: var(--surface-hover); }
.tile-on { background-color: var(--accent-subtle, rgba(80,140,255,0.18)); }
.tile-pending { opacity: 0.45; }
.ai-diff { padding: 8px 10px; border-radius: 8px; background-color: var(--primary-soft); }
.graph-thumb { background-color: var(--surface-sunken, #15171c); padding: 8px; }
.graph-chain { font-size: 11px; color: var(--text-subtle); text-align: center; }
.tile-thumb { flex-grow: 1; border-radius: 8px; background-color: #000000; overflow: hidden; }
.head-icon { icon-size: 16px; icon-color: var(--text-subtle); }
.tile-icon { icon-size: 30px; icon-color: var(--text-subtle); }
.tile-badge { margin: 4px; padding: 1px 6px; border-radius: 6px; background-color: #000000B0; }
.tile-badge-text { font-size: 10px; color: #FFFFFF; font-family: monospace; }
.tile-scrub { background-color: var(--warning); }

/* ── уведомления (как в Synthos) ── */
Notification {
    /* Контент-driven ширина: max 25% viewport (syngui резолвит percent
     * относительно containing-block, передаваемого Portal'ом). */
    max-width: 25%;
    gap: 12px;
    padding: 14px 16px;
    border-radius: 12px;
    background-color: var(--bg-shell);
    color: var(--text);
    font-size: 14px;
    /* Transitions для opacity/transform применяются на каждом frame'е через
     * fade animation в element. Эти MSS значения — будущая интеграция
     * с keyframe slide-in справа. */
    transition: opacity 200ms ease-out,
                transform 200ms ease-out;
}

/* Severity accent colors — переопределяют fallback из syngui.
 * Виджет читает `accent-color` для левого бордера каждой карточки. */
Notification.severity-info    { accent-color: var(--primary, #3B82F6); }
Notification.severity-success { accent-color: #22C55E; }
Notification.severity-warning { accent-color: #F59E0B; }
Notification.severity-error   { accent-color: var(--error, #EF4444); }

.graph-head { padding: 4px 8px; border-bottom-width: 1px; border-color: var(--border-soft); }
.graph-tab { height: 26px; padding: 0 6px 0 10px; border-radius: 7px; cursor: pointer; }
.graph-tab:hover { background-color: var(--surface-hover); }
.graph-tab-on { background-color: var(--bg-panel-2); border-width: 1px; border-color: var(--border); }
.graph-tab-text { font-size: 12px; color: var(--text-muted); }
.graph-tab-on .graph-tab-text { color: var(--text); }
.graph-tab-x { icon-size: 13px; icon-color: var(--text-subtle); }

/* ── шапка графа: масштаб и прогон в стиле панели инструментов ── */
.graph-zoom .node-editor-toolbar-btn { border-radius: 6px; color: var(--text-muted); }
.graph-zoom .node-editor-toolbar-btn:hover { background-color: var(--surface-hover); color: var(--text); }
.graph-zoom .node-editor-toolbar-zoom { color: var(--text); font-weight: 500; }
.graph-head .ne-run-controls { background-color: transparent; border-width: 0px; box-shadow: none; padding: 0px; }
.graph-head .ne-run-controls--running { background-color: var(--success-soft); border-radius: 8px; }
.graph-head .ne-run-controls--paused { background-color: var(--warning-soft); border-radius: 8px; }
.graph-head .ne-run-btn { border-radius: 6px; color: var(--text-muted); background-color: transparent; }
.graph-head .ne-run-btn:hover { background-color: var(--surface-hover); color: var(--text); }
.graph-head .ne-run-btn--play.ne-run-btn--active { background-color: var(--success-soft); color: var(--success); box-shadow: none; }
.graph-head .ne-run-btn--pause.ne-run-btn--active { background-color: var(--warning-soft); color: var(--warning); }
.graph-head .ne-run-btn--stop.ne-run-btn--active { background-color: var(--surface-hover); color: var(--error); }
.clip-drop { background-color: var(--primary-soft); border-width: 2px; border-color: var(--primary); }
.tl-head-new { justify-content: center; }
.tl-head-new .tl-head-text { color: var(--text-subtle); font-size: 11px; font-weight: 400; }
.tl-row-new { border-bottom-width: 1px; border-color: var(--border-soft); background-color: #FFFFFF05; }
.clip-drop-new { border-style: dashed; }
.clip-edge { cursor: ew-resize; border-radius: 4px; }
.clip-edge:hover { background-color: #FFFFFF55; }
.clip-source { border-radius: 6px; background-color: #FFFFFF14; border-width: 1px; border-color: #FFFFFF33; }
.clip-trim { background-color: var(--primary-soft); border-width: 2px; border-color: var(--warning); }
.tl-ruler-hit { background-color: transparent; }
.tl-playhead-hit { background-color: transparent; }
.transport { padding: 4px 10px; border-top-width: 1px; border-color: var(--border-soft); background-color: var(--bg-panel); }
.transport-time { font-family: monospace; font-size: 12px; color: var(--text-muted); }
.transport-play { icon-size: 22px; }
.clip-frame { border-right-width: 1px; border-color: #00000055; }
.clip-label { padding: 1px 6px; margin: 3px 4px; border-radius: 4px; background-color: #00000080; }
.fx-head { padding: 8px 8px 2px 8px; border-top-width: 1px; border-color: var(--border-soft); margin-top: 4px; }
.fx-title { font-size: 12px; font-weight: 600; color: var(--text); }
.tl-transition { border-radius: 11px; background-color: #FFFFFFD0; border-width: 1px; border-color: var(--primary); cursor: pointer; }
.tl-transition:hover { background-color: #FFFFFF; }
.tl-transition-text { font-size: 10px; font-weight: 600; color: #1A1B22; }
.tl-transition-edge { cursor: ew-resize; }
.tl-transition-edit { background-color: var(--primary); }
.tl-transition-edit .tl-transition-text { color: var(--on-primary); }
.insp-grid { width: 100%; background-color: var(--bg-panel); border-color: var(--border-soft); color: var(--text); accent-color: var(--primary); font-size: 12px; }
.tl-bar { padding: 3px 8px; border-bottom-width: 1px; border-color: var(--border-soft); background-color: var(--bg-panel); }
.tl-bar-hint { font-size: 11px; color: var(--text-muted); }
.tl-band { background-color: #3B82F622; border-width: 1px; border-color: var(--primary); border-radius: 2px; }
.tl-snap { background-color: #FFD54F; }
.tl-head-off { opacity: 0.55; }
.tl-head-edit { font-size: 12px; height: 24px; }
.clip-off { opacity: 0.4; }
.fp-scrim { background-color: #00000088; }
.fp-card { background-color: var(--bg-panel); border-radius: 12px; border-width: 1px; border-color: var(--border-soft); padding: 14px; }
.fp-title { font-size: 15px; font-weight: 700; color: var(--text); }
.fp-count { font-size: 11px; color: var(--text-muted); }
.fp-row { padding: 8px 12px; border-radius: 8px; }
.fp-row:hover { background-color: var(--bg-hover, #FFFFFF10); }
.fp-row-on { background-color: #3B82F633; }
.fp-sample { font-size: 24px; color: var(--text); }
.fp-name { font-size: 11px; color: var(--text-muted); }
.clip-key { font-size: 10px; color: #FFD54F; }
.curves { padding: 8px 10px; }
.cv-area { background-color: var(--bg-panel); border-radius: 8px; border-width: 1px; border-color: var(--border-soft); }
.cv-chip { height: 22px; padding: 0 9px; border-radius: 11px; border-width: 1px; border-color: var(--border-soft); justify-content: center; }
.cv-chip:hover { background-color: var(--surface-hover); }
.cv-chip-on { background-color: var(--primary-soft); border-color: var(--primary); }
.cv-chip-text { font-size: 11px; color: var(--text); }
.cv-hint { font-size: 11px; color: var(--text-muted); }
.cv-axis { font-size: 10px; color: var(--text-muted); font-family: monospace; }
.layer-row { padding: 4px 8px; border-radius: 6px; cursor: pointer; }
.layer-row:hover { background-color: var(--surface-hover); }
.layer-row-on { background-color: var(--primary-soft); border-width: 1px; border-color: var(--primary); }
.layer-row-off { opacity: 0.5; }
.layer-kind { font-size: 12px; color: var(--primary); width: 16px; }
.layer-badge { font-size: 10px; color: var(--text-muted); font-family: monospace; }
.layer-key { font-size: 10px; color: #FFD54F; font-family: monospace; }
.lp-label { font-size: 12px; color: var(--text-muted); }
.lp-item { padding: 10px 12px; border-radius: 10px; background-color: var(--bg-panel-2); border-width: 1px; border-color: var(--border-soft); }
.lp-item-on { border-color: var(--primary); }
.lp-name { font-size: 13px; font-weight: 600; color: var(--text); }
.lp-badge { padding: 1px 7px; border-radius: 999px; background-color: var(--surface-hover); }
.lp-badge-on { background-color: var(--primary-soft); }
.lp-badge-text { font-size: 10px; color: var(--text-muted); }
.lp-badge-on .lp-badge-text { color: var(--primary); }
.lp-desc { font-size: 12px; color: var(--text-muted); }
.lp-path { font-size: 10px; color: var(--text-subtle); font-family: monospace; }

/* ── галерея шаблонов (C4) ── */
.tpl-search { height: 28px; font-size: 12px; }
.tpl-cats { padding: 6px 8px; border-bottom-width: 1px; border-color: var(--border-soft); }
.tpl-sub-select { height: 26px; font-size: 11px; }
.tpl-more { padding: 10px 0 14px 0; }

/* Панель «Цвет» (F2) */
.color-panel { padding: 8px; flex-grow: 1; }
.cg-title { font-size: 12px; font-weight: 600; color: var(--text); }
.cg-label { font-size: 11px; color: var(--text-muted); }
.cg-value { font-size: 12px; color: var(--text); }
.cmp-line { background-color: rgba(255, 255, 255, 0.85); }
.cmp-tag { background-color: rgba(0, 0, 0, 0.6); border-radius: 4px; padding: 2px 6px; }
.cmp-tag-text { font-size: 11px; color: #ffffff; }

/* настройки: редактор скилов и сценариев */
.set-edit-list { width: 250px; padding: 10px 8px; background-color: var(--bg-panel); border-radius: 12px; }
.set-code { height: 640px; border-radius: 8px; border: 1px solid var(--border); }
.set-code-prompt { height: 460px; border-radius: 8px; border: 1px solid var(--border); }
.set-status { font-size: 12px; color: var(--success); }
.set-status.bad { color: var(--danger); }
.mono { font-family: monospace; }
.pv-fill { width: 100%; height: 100%; }

/* панель «Проекты» — карточки недавних проектов */
.proj-card { border-radius: 12px; border-width: 1px; border-color: var(--border-soft); background-color: var(--bg-panel-2); overflow: hidden; cursor: pointer; }
.proj-card:hover { border-color: var(--primary); background-color: var(--surface-hover); }
.proj-on { border-color: var(--primary); border-width: 2px; }
.proj-cover { flex-grow: 1; background-color: #000000; overflow: hidden; }
.proj-body { padding: 10px 12px 12px 12px; }
.proj-title { font-size: 14px; font-weight: 600; color: var(--text); }
.proj-chip { padding: 2px 8px; border-radius: 999px; background-color: var(--surface-hover); }
.proj-chip-icon { icon-size: 13px; icon-color: var(--text-muted); }
.proj-chip-text { font-size: 11px; color: var(--text-muted); }
.proj-empty-icon { icon-size: 36px; icon-color: var(--text-subtle); }
.proj-open-badge { padding: 1px 8px; border-radius: 6px; background-color: var(--primary); }
.proj-open-text { font-size: 10px; font-weight: 600; color: var(--on-primary); }
.proj-dot { width: 14px; height: 3px; border-radius: 2px; background-color: #FFFFFF55; }
.proj-dot-on { background-color: #FFFFFF; }
.media-chips { padding: 6px 8px 2px 8px; }
.media-folder { padding: 4px 8px 0 8px; }
