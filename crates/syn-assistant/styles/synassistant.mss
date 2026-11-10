.grow { flex-grow: 1; }
.ic { icon-size: 20px; icon-color: var(--muted); }
.hint { font-size: 12px; color: var(--muted); }
.pad { padding: 10px; }

/* ── Окно ─────────────────────────────────────────────────────────── */

.window { background: var(--content); color: var(--fg); font-size: 13px; }
.window.phone { background: var(--content); }
.window-frame { border-radius: var(--window-radius); border: 1px solid var(--border); }
.window-frame:window-maximized, .window-frame:window-fullscreen { border-radius: 0px; border-width: 0px; }
.window-controls { height: 52px; padding: 0px 6px 0px 4px; align-items: center; }
SystemWindowControls { color: var(--fg); background-color: var(--hover); }

.brand { width: 28px; height: 28px; border-radius: 9px; background: linear-gradient(135deg, var(--accent), var(--accent-2)); justify-content: center; align-items: center; }
.brand-ic { icon-color: #ffffff; icon-size: 17px; }
.brand-title { font-size: 14px; font-weight: 600; color: var(--fg); }

/* Шапка панели чата — заголовок окна. */
.pane-header { height: 52px; padding: 0px 0px 0px 12px; }
.hd-drag { height: 52px; flex-grow: 1; min-width: 40px; align-items: center; padding: 0px 8px; }
.hd-title-row { height: 52px; }
.hd-title { font-size: 14px; font-weight: 600; color: var(--fg); max-width: 640px; }
.tb-title { font-size: 14px; font-weight: 600; color: var(--fg); }
.phone-bar { height: 52px; padding: 4px 8px; }
.bar-drag { height: 44px; align-items: center; }
.head-drag { height: 36px; flex-grow: 1; min-width: 20px; }
.usage-box { padding: 0px 8px; }
.usage { width: 56px; height: 4px; border-radius: 999px; color: var(--accent); background-color: var(--surface); }
.usage.warn { color: var(--warning); }
.usage-label { font-size: 11px; color: var(--muted); }

/* ── Кнопки ───────────────────────────────────────────────────────── */

.btn { padding: 8px 12px; border-radius: 10px; background-color: var(--surface); color: var(--fg); font-size: 13px; transition: background-color 120ms ease-out; }
.btn:hover { background-color: var(--hover); }
.btn:pressed { background-color: var(--pressed); }
.btn.primary { padding: 8px 14px; background-color: var(--accent); color: var(--accent-fg); icon-color: var(--accent-fg); }
.btn.primary:hover { background-color: var(--accent-hover); }
.btn.send { width: 34px; height: 34px; padding: 0px; justify-content: center; align-items: center; background-color: var(--accent); icon-size: 20px; icon-color: var(--accent-fg); }
.btn.send:hover { background-color: var(--accent-hover); }
.btn.stop { width: 34px; height: 34px; padding: 0px; justify-content: center; align-items: center; background-color: var(--fg); icon-size: 18px; icon-color: var(--content); }
.btn.mic { width: 34px; height: 34px; padding: 0px; justify-content: center; align-items: center; background-color: #00000000; icon-size: 20px; icon-color: var(--muted); }
.btn.mic:hover { background-color: var(--hover); icon-color: var(--fg); }
.btn.mic.on { background-color: var(--danger); icon-color: #ffffff; glow: 0 0 14px var(--danger); }
.btn.mic.off { opacity: 0.5; }
.btn.attach { width: 34px; height: 34px; padding: 0px; justify-content: center; align-items: center; icon-size: 20px; icon-color: var(--muted); }
.btn.attach:hover { icon-color: var(--fg); }
.btn.voice-chip { padding: 6px 12px; border-radius: 999px; font-size: 12px; background-color: #00000000; border: 1px solid var(--border); icon-size: 17px; icon-color: var(--accent); }
.btn.voice-chip:hover { background-color: var(--hover); }
.btn.voice-chip.on { background-color: var(--accent); border-color: var(--accent); color: var(--accent-fg); icon-color: var(--accent-fg); }
.btn.voice-start { padding: 11px 20px; border-radius: 999px; font-size: 14px; background-color: var(--accent); color: var(--accent-fg); icon-color: var(--accent-fg); }
.btn.voice-start:hover { background-color: var(--accent-hover); }
.btn.ghost { background-color: #00000000; border: 1px solid var(--border); }
.btn.ghost:hover { background-color: var(--hover); }
.btn.small { padding: 6px 10px; font-size: 12px; }
.btn.new { width: 100%; padding: 9px 12px; border-radius: 10px; background-color: #00000000; border: 1px solid var(--border); color: var(--fg); icon-size: 18px; icon-color: var(--fg); }
.btn.new:hover { background-color: var(--hover); }
.btn.round { padding: 8px; border-radius: 999px; }
.btn.flat { background-color: #00000000; }
.btn.flat:hover { background-color: var(--hover); }
.btn.tb-btn { padding: 7px; icon-size: 20px; icon-color: var(--muted); }
.btn.tb-btn:hover { icon-color: var(--fg); }
.btn.tiny { padding: 3px; icon-size: 16px; icon-color: var(--muted); }
.more-gap { width: 22px; height: 22px; }
.btn.act { padding: 5px; icon-size: 16px; icon-color: var(--muted); }
.btn.act:hover { icon-color: var(--fg); }
.btn.chip { padding: 5px 10px; border-radius: 999px; font-size: 12px; background-color: #00000000; color: var(--muted); icon-size: 16px; icon-color: var(--muted); }
.btn.chip:hover { background-color: var(--hover); color: var(--fg); }
.btn.chip.on { background-color: var(--accent-dim); color: var(--accent); icon-color: var(--accent); }
.btn.chip.speaking { background-color: var(--danger-soft); color: var(--danger); icon-color: var(--danger); }
.btn.example { padding: 14px 16px; border-radius: 14px; font-size: 13px; background-color: #00000000; border: 1px solid var(--border); icon-size: 18px; icon-color: var(--accent); }
.btn.example:hover { background-color: var(--hover); }
.btn.att { padding: 5px 10px; font-size: 12px; border-radius: 8px; background-color: var(--surface-alt); icon-size: 16px; }

/* ── Боковая панель ───────────────────────────────────────────────── */

.sidebar { width: 100%; height: 100%; padding: 8px 8px 8px 10px; background: var(--sidebar); }
/* Разделитель панели чатов: тонкая линия, при наведении и перетаскивании — акцент. */
.side-split { border-color: var(--divider); accent-color: var(--accent); divider-thickness: 1px; }
.sidebar.phone { width: 100%; flex-grow: 1; border-width: 0px; }
.sidebar-head { height: 36px; padding: 0px 0px 0px 2px; }
TextField.search { background-color: #00000000; border: 1px solid var(--divider); border-radius: 10px; padding: 7px 10px; font-size: 13px; color: var(--fg); }
TextField.search:focus { border-color: var(--accent); }
TextField.rename-field { background-color: var(--field); border: 1px solid var(--accent); border-radius: 8px; padding: 6px 10px; font-size: 13px; color: var(--fg); }
.chat-list { padding: 0px 2px 0px 0px; }
.group-title { font-size: 11px; font-weight: 600; color: var(--muted); padding: 16px 10px 6px 10px; }
.chat-item { min-height: 36px; padding: 0px 6px 0px 10px; border-radius: 9px; transition: background-color 120ms ease-out; }
.chat-item:hover { background-color: var(--hover); }
.chat-item.active { background-color: var(--accent-dim); }
.chat-item.renaming { padding: 2px; }
.chat-title { font-size: 13px; color: var(--fg); opacity: 0.86; }
.chat-title.on { opacity: 1; font-weight: 600; }
.chat-time { font-size: 11px; color: var(--muted); padding: 0px 4px 0px 0px; }
.status { padding: 8px 2px 0px 6px; border-top: 1px solid var(--divider); min-height: 44px; }
.svc { padding: 4px 6px; border-radius: 8px; }
.svc:hover { background-color: var(--hover); }
.svc-ic { icon-size: 15px; }
.svc.off .svc-ic { opacity: 0.5; }
.dot { width: 6px; height: 6px; border-radius: 999px; }
.dot.ok { background-color: var(--success); }
.dot.off { background-color: var(--danger); }
.status-label { font-size: 11px; color: var(--muted); }

/* ── Лента ────────────────────────────────────────────────────────── */

.pane-wrap { background: var(--content); }
.center-host { width: 100%; }
.column { width: 100%; max-width: 860px; }
.feed-pad { padding: 12px 28px 24px 28px; }
.user-row { padding: 6px 0px 0px 0px; }
.user-bubble { padding: 10px 16px; border-radius: 20px; background-color: var(--surface); max-width: 600px; }
.user-text { color: var(--fg); font-size: 14px; }
.assistant { }
.answer { font-size: 14px; color: var(--fg); }
/* Markdown ответов — цвета темы. */
.answer, .think-body, .tool-out {
    --md-heading-color: var(--fg); --md-link-color: var(--accent); --md-bullet-color: var(--muted); --md-checkbox-color: var(--accent);
    --md-code-bg: var(--surface); --md-code-color: var(--fg); --md-code-block-bg: var(--code-bg); --md-code-block-color: var(--fg);
    --md-quote-bg: var(--accent-dim); --md-quote-text-color: var(--fg); --md-quote-border-color: var(--accent);
    --md-table-border-color: var(--border); --md-table-header-bg: var(--surface); --md-table-header-color: var(--fg); --md-table-stripe-bg: var(--hover);
    --md-hr-color: var(--divider);
}
.actions { padding: 0px 0px 0px 0px; margin: 0px 0px 0px -5px; }
.think { padding: 8px 12px; border-radius: 12px; border: 1px solid var(--divider); }
.think-ic { icon-size: 16px; icon-color: var(--muted); }
.think-head { font-size: 12px; color: var(--muted); }
.think-body { font-size: 12px; color: var(--muted); }

/* Шаги — вызовы инструментов: компактный список строк. */
.steps { padding: 4px; border-radius: 12px; border: 1px solid var(--divider); }
.step { }
.step-row { min-height: 34px; padding: 6px 8px; border-radius: 8px; transition: background-color 120ms ease-out; }
.step-row:hover { background-color: var(--hover); }
.step-row.open { background-color: var(--hover); }
.step-ic { icon-size: 16px; icon-color: var(--accent); }
.step-ic.fail { icon-color: var(--danger); }
.step-sum { font-size: 13px; color: var(--fg); }
.step-name { font-family: monospace; font-size: 11px; color: var(--muted); }
.step-count { font-size: 11px; color: var(--muted); }
.step-ok { icon-size: 15px; icon-color: var(--success); }
.step-fail { icon-size: 15px; icon-color: var(--danger); }
.step-chevron { icon-size: 18px; icon-color: var(--muted); }


/* Сводка сжатого начала разговора. */
.tool-card { padding: 8px 12px; border-radius: 12px; border: 1px solid var(--divider); }
.tool-badge { width: 24px; height: 24px; border-radius: 7px; background-color: var(--accent-dim); justify-content: center; align-items: center; }
.tool-ic { icon-size: 15px; icon-color: var(--accent); }
.tool-sum { font-size: 13px; color: var(--muted); }
.tool-out { font-size: 12px; color: var(--muted); }

/* Окно вывода инструмента: тёмный терминал в любой теме. */
.term-wrap { padding: 0px 4px 6px 34px; }
.step-files { padding: 2px 4px 6px 34px; }
.term { border-radius: 10px; background-color: #0B0E14; border: 1px solid #FFFFFF14; }
.term-bar { padding: 4px 6px 4px 10px; min-height: 30px; background-color: #151A23; border-bottom-width: 1px; border-color: #FFFFFF0F; align-items: center; }
.term-dot { width: 9px; height: 9px; border-radius: 5px; background-color: #3A404C; }
.term-dot.r { background-color: #FF5F57; }
.term-dot.y { background-color: #FEBC2E; }
.term-dot.g { background-color: #28C840; }
.term-cmd { font-family: monospace; font-size: 11px; color: #8B949E; padding: 0px 0px 0px 6px; }
.term-state { font-size: 10px; color: #6E7681; }
.term-state.run { color: #3FB950; }
.term-state.fail { color: #FF7B72; }
.btn.term-copy { padding: 4px; icon-size: 14px; icon-color: #8B949E; }
.btn.term-copy:hover { background-color: #FFFFFF14; icon-color: #C9D1D9; }
.term-body { padding: 8px 12px 10px 12px; }
.term-line { font-family: monospace; font-size: 12px; color: #C9D1D9; }
.term-line.err { color: #FF7B72; }
.term-note { font-family: monospace; font-size: 12px; color: #6E7681; }
.term-cursor { font-family: monospace; font-size: 12px; color: #3FB950; }

.att-tile-box { border-radius: 12px; border-width: 1px; border-color: var(--border); background-color: var(--surface-alt); }
.att-tile { width: 100%; height: 100%; border-radius: 12px; }
.att-tile-empty { background-color: var(--surface-alt); }
.att-play { width: 36px; height: 36px; border-radius: 999px; background-color: #000000aa; justify-content: center; align-items: center; }
.att-play-ic { icon-size: 24px; icon-color: #ffffff; }
.btn.att-x { width: 22px; height: 22px; padding: 0px; border-radius: 999px; background-color: #000000aa; icon-size: 14px; color: #ffffff; icon-color: #ffffff; justify-content: center; align-items: center; }
.btn.att-x:hover { background-color: var(--danger); }
.btn.att-x-inline { padding: 4px; border-radius: 999px; background-color: #00000000; icon-size: 14px; color: var(--muted); icon-color: var(--muted); }
.att-image { max-width: 420px; max-height: 420px; border-radius: 12px; }
.typing { accent-color: var(--accent); }
.spin { color: var(--accent); }
.error-card { padding: 10px 12px; border-radius: 12px; background-color: var(--danger-soft); border: 1px solid var(--danger); }
.error-ic { icon-color: var(--danger); }
.error { font-size: 13px; color: var(--fg); }

.confirm { margin: 0px 28px 8px 28px; padding: 12px 14px; border-radius: 14px; background-color: var(--field); border: 1px solid var(--accent); box-shadow: 0 6px 20px var(--shadow); }
.confirm.voice { margin: 0px; max-width: 560px; }
.confirm-badge { width: 36px; height: 36px; border-radius: 10px; background-color: var(--accent-dim); justify-content: center; align-items: center; }
.confirm-ic { icon-size: 22px; icon-color: var(--accent); }
.confirm-title { font-size: 13px; font-weight: 600; color: var(--fg); }
.confirm-sum { font-size: 12px; color: var(--fg); font-family: monospace; }

/* ── Ввод ─────────────────────────────────────────────────────────── */

.compose-wrap { padding: 4px 28px 20px 28px; }
.composer { padding: 10px 10px 8px 12px; border-radius: 22px; background-color: var(--field); border: 1px solid var(--border); box-shadow: 0 10px 30px var(--shadow); transition: border-color 150ms ease-out; }
.composer:focus-within { border-color: var(--accent-soft); }
MultilineTextEdit.input { background-color: #00000000; color: var(--fg); border: 0px solid #00000000; padding: 6px 4px 8px 4px; font-size: 14px; caret-color: var(--accent); }
.composer-bar { min-height: 36px; }
Dropdown.model-pick { background-color: #00000000; border-radius: 999px; padding: 5px 10px; font-size: 12px; color: var(--muted); border-width: 0px; }
Dropdown.model-pick:hover { background-color: var(--hover); color: var(--fg); }
.att-row { padding: 2px 4px; }
.att-chip { font-size: 12px; background-color: var(--accent-dim); color: var(--fg); }
.dict-row { padding: 4px 8px; }
.dict-text { font-size: 12px; color: var(--muted); }
.wave { accent-color: var(--accent); }
.warn-ic { icon-size: 16px; icon-color: var(--warning); }
.warn-text { font-size: 12px; color: var(--muted); }

/* ── Пустой чат ───────────────────────────────────────────────────── */

.empty { padding: 32px 24px 24px 24px; }
.hero { width: 100%; max-width: 620px; }
.orb { accent-color: var(--accent); color: var(--accent-2); }
.hero-title { font-size: 28px; font-weight: 600; color: var(--fg); }
.hero-sub { font-size: 14px; color: var(--muted); max-width: 520px; text-align: center; }
.examples { width: 100%; max-width: 600px; padding: 8px 0px; }

/* ── Голосовой режим ──────────────────────────────────────────────── */

.voice-scrim { width: 100%; height: 100%; background-color: var(--scrim); backdrop-filter: blur(18px); }
.voice-host { width: 100%; height: 100%; padding: 24px; }
.voice-card { width: 100%; max-width: 640px; padding: 12px; }
.aura-wrap { width: 260px; height: 260px; }
.aura { accent-color: var(--accent); color: var(--accent-2); }
.voice-center-slot { width: 260px; height: 260px; }
.btn.voice-center { width: 96px; height: 96px; padding: 0px; border-radius: 999px; background-color: #00000000; icon-size: 40px; icon-color: var(--accent-fg); justify-content: center; align-items: center; }
.btn.voice-center:hover { background-color: #ffffff22; }
.voice-status { font-size: 16px; font-weight: 600; color: var(--fg); text-align: center; }
.voice-transcript { width: 100%; max-width: 600px; }
.voice-answer-scroll { height: 200px; }
.voice-answer { font-size: 15px; color: var(--fg); }
.btn.voice-act { padding: 14px; background-color: var(--surface); icon-size: 24px; icon-color: var(--fg); }
.btn.voice-act:hover { background-color: var(--hover); }
.btn.voice-act.off { background-color: var(--danger-soft); icon-color: var(--danger); }
.btn.voice-act.end { background-color: var(--danger); icon-color: #ffffff; }

/* ── Слои ─────────────────────────────────────────────────────────── */

.toast-wrap { width: 100%; height: 100%; padding: 0px 0px 96px 0px; }
.toast { padding: 10px 16px; border-radius: 12px; background-color: var(--menu-bg); border: 1px solid var(--border); box-shadow: 0 8px 24px var(--shadow); }
.toast-ic { icon-size: 18px; icon-color: var(--success); }
.toast-text { font-size: 13px; color: var(--fg); }

/* ─── Рецепты ─── */
.recipe-detail { padding: 22px 28px; max-width: 900px; }
.recipe-empty { padding: 40px; }
.recipe-empty-ic { icon-size: 48px; icon-color: var(--accent); }
.recipe-title { font-size: 20px; font-weight: 700; color: var(--fg); }
.recipe-desc { font-size: 14px; color: var(--fg); }
.recipe-section { font-size: 13px; font-weight: 700; color: var(--muted); padding-top: 4px; }
.recipe-hint { font-size: 12px; color: var(--muted); }
.recipe-param { font-size: 13px; font-weight: 600; color: var(--fg); }
.recipe-field { padding: 7px 10px; border-radius: 10px; border-width: 1px; border-color: var(--border); background-color: var(--surface-alt); font-size: 13px; color: var(--fg); }
.recipe-step { padding: 8px 12px; border-radius: 10px; background-color: var(--surface-alt); }
.recipe-step-n { font-size: 12px; font-weight: 700; color: var(--accent); min-width: 16px; }
.recipe-step-ic { icon-size: 18px; icon-color: var(--accent); }
.recipe-step-title { font-size: 13px; color: var(--fg); }
.recipe-log { padding: 4px 0px; }
.recipe-error { font-size: 13px; color: var(--danger); }

.tool-progress-box { padding: 0px 4px 2px 36px; }
.tool-progress { height: 6px; border-radius: 3px; background-color: var(--surface-alt); color: var(--accent); accent-color: var(--accent); }

.player-audio { padding: 8px 12px; border-radius: 12px; background-color: var(--surface-alt); border-width: 1px; border-color: var(--border); }
.player-name { font-size: 12px; color: var(--fg); }
.btn.round.play-btn { padding: 8px; background-color: var(--accent); icon-color: var(--accent-fg); color: var(--accent-fg); }
.btn.round.play-btn:hover { background-color: var(--accent); icon-color: var(--accent-fg); }
.player-video { border-radius: 12px; }
