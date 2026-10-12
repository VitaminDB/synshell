.grow { flex-grow: 1; }
.ic { icon-size: 20px; icon-color: var(--muted); }
.hint { font-size: 12px; color: var(--muted); }
.pad { padding: 10px; }

/* ── Окно ─────────────────────────────────────────────────────────── */

.window { background: var(--content); color: var(--fg); font-size: 13px; }
.window.phone { background: var(--bg); }
.window-frame {
    border-radius: var(--window-radius);
    border: 1px solid var(--border);
}
.window-frame:window-maximized, .window-frame:window-fullscreen { border-radius: 0px; border-width: 0px; }

.titlebar { height: 46px; background: var(--titlebar); }
.tb-left-wrap { height: 46px; padding: 0px 6px 0px 8px; align-items: center; }
.tb-left { height: 46px; align-items: center; }
.tb-left.open { width: 260px; }
.drag-wrap { height: 46px; }
.drag-space { height: 46px; flex-grow: 1; min-width: 40px; align-items: center; justify-content: center; }
.tb-title-row { height: 46px; }
.tb-title { font-size: 13px; font-weight: 600; color: var(--fg); max-width: 520px; }
.tb-tools { height: 46px; padding: 0px 6px 0px 6px; align-items: center; }
.window-controls { height: 46px; padding: 0px 4px 0px 4px; align-items: center; }
SystemWindowControls { color: var(--fg); background-color: var(--hover); }

.brand { width: 28px; height: 28px; border-radius: 9px; background: linear-gradient(135deg, var(--accent), var(--accent-2)); justify-content: center; align-items: center; }
.brand-ic { icon-color: #ffffff; icon-size: 18px; }
.brand-title { font-size: 14px; font-weight: 600; color: var(--fg); }

.phone-bar { height: 52px; padding: 4px 8px; background: var(--titlebar); }

/* ── Кнопки ───────────────────────────────────────────────────────── */

.btn { padding: 8px 12px; border-radius: 10px; background-color: var(--surface); color: var(--fg); font-size: 13px; transition: background-color 120ms ease-out; }
.btn:hover { background-color: var(--hover); }
.btn:pressed { background-color: var(--pressed); }
.btn.primary { padding: 8px 14px; background: linear-gradient(135deg, var(--accent), var(--accent-2)); color: var(--accent-fg); icon-color: var(--accent-fg); }
.btn.primary:hover { background: linear-gradient(135deg, var(--accent-2), var(--accent)); }
.btn.send { padding: 9px; background: linear-gradient(135deg, var(--accent), var(--accent-2)); icon-color: var(--accent-fg); box-shadow: 0 4px 12px var(--accent-glow); }
.btn.send:hover { background: linear-gradient(135deg, var(--accent-2), var(--accent)); }
.btn.voice-chip.on { background: linear-gradient(135deg, var(--accent), var(--accent-2)); color: var(--accent-fg); icon-color: var(--accent-fg); }
.btn.voice-chip.on:hover { background: linear-gradient(135deg, var(--accent-2), var(--accent)); }
.btn.voice-start { padding: 12px 20px; border-radius: 999px; font-size: 14px; background: linear-gradient(135deg, var(--accent), var(--accent-2)); color: var(--accent-fg); icon-color: var(--accent-fg); box-shadow: 0 6px 20px var(--accent-glow); }
.btn.voice-start:hover { background: linear-gradient(135deg, var(--accent-2), var(--accent)); }
.btn.ghost { background-color: #00000000; border: 1px solid var(--border); }
.btn.ghost:hover { background-color: var(--hover); }
.btn.small { padding: 6px 10px; font-size: 12px; }
.btn.new { padding: 10px 12px; border-radius: 12px; background: var(--accent-dim); color: var(--fg); icon-color: var(--accent); }
.btn.new:hover { background: var(--accent-soft); }
.btn.round { padding: 8px; border-radius: 999px; }
.btn.flat { background-color: #00000000; }
.btn.flat:hover { background-color: var(--hover); }
.btn.tb-btn { padding: 7px; icon-size: 20px; icon-color: var(--muted); }
.btn.tb-btn:hover { icon-color: var(--fg); }
.btn.tiny { padding: 3px; icon-size: 16px; icon-color: var(--muted); }
.btn.act { padding: 5px; icon-size: 16px; icon-color: var(--muted); }
.btn.act:hover { icon-color: var(--fg); }
.btn.stop { padding: 9px; background-color: var(--danger); color: #ffffff; icon-color: #ffffff; }
.btn.mic { padding: 9px; background-color: var(--surface); icon-color: var(--fg); }
.btn.mic.on { background-color: var(--danger); icon-color: #ffffff; glow: 0 0 14px var(--danger); }
.btn.mic.off { icon-color: var(--muted); opacity: 0.6; }
.btn.attach { padding: 9px; icon-color: var(--muted); }
.btn.chip { padding: 6px 10px; border-radius: 999px; font-size: 12px; background-color: var(--surface); icon-size: 16px; icon-color: var(--muted); }
.btn.chip:hover { background-color: var(--hover); }
.btn.chip.on { background-color: var(--accent-soft); color: var(--fg); icon-color: var(--accent); }
.btn.chip.speaking { background-color: var(--danger-soft); icon-color: var(--danger); }
.btn.voice-chip { padding: 6px 12px; border-radius: 999px; font-size: 12px; background-color: var(--accent-dim); icon-size: 18px; icon-color: var(--accent); }
.btn.voice-chip:hover { background-color: var(--accent-soft); }
.btn.example { padding: 12px 14px; border-radius: 14px; font-size: 13px; background-color: var(--card); border: 1px solid var(--divider); icon-size: 18px; icon-color: var(--accent); }
.btn.example:hover { background-color: var(--card-hover); }
.btn.att { padding: 5px 10px; font-size: 12px; border-radius: 8px; background-color: var(--surface-alt); icon-size: 16px; }

/* ── Боковая панель ───────────────────────────────────────────────── */

.sidebar { width: 272px; padding: 8px 10px 10px 10px; background: var(--sidebar); }
.sidebar.phone { width: 100%; flex-grow: 1; }
.sidebar-head { height: 36px; padding: 0px 2px; }
TextField.search { background-color: var(--field); border: 1px solid var(--border); border-radius: 10px; padding: 7px 10px; font-size: 13px; color: var(--fg); }
TextField.search:focus { border-color: var(--accent); }
TextField.rename-field { background-color: var(--field); border: 1px solid var(--accent); border-radius: 8px; padding: 6px 10px; font-size: 13px; color: var(--fg); }
.chat-list { padding: 0px 2px 0px 0px; }
.group-title { font-size: 11px; font-weight: 600; color: var(--muted); padding: 10px 10px 4px 10px; }
.chat-item { padding: 8px 6px 8px 10px; border-radius: 10px; transition: background-color 120ms ease-out; }
.chat-item:hover { background-color: var(--hover); }
.chat-item.active { background-color: var(--accent-dim); }
.chat-item.renaming { padding: 2px; }
.chat-ic { icon-size: 18px; icon-color: var(--muted); }
.chat-ic.on { icon-color: var(--accent); }
.chat-title { font-size: 13px; color: var(--fg); }
.chat-preview { font-size: 11px; color: var(--muted); }
.status { padding: 8px 6px 0px 6px; border-top: 1px solid var(--divider); }
.dot { width: 8px; height: 8px; border-radius: 999px; }
.dot.ok { background-color: var(--success); glow: 0 0 6px var(--success); }
.dot.off { background-color: var(--muted); opacity: 0.5; }
.status-label { font-size: 11px; color: var(--muted); min-width: 48px; }
.status-value { font-size: 11px; color: var(--fg); }

/* ── Панель чата ──────────────────────────────────────────────────── */

.pane-wrap { background: var(--content); }
.toolbar { height: 48px; padding: 6px 14px 6px 14px; border-bottom: 1px solid var(--divider); }
.toolbar.phone { padding: 6px 8px; }
Dropdown.model-pick.phone { max-width: 180px; }
Dropdown.model-pick { background-color: var(--surface); border-radius: 999px; padding: 6px 12px; font-size: 12px; color: var(--fg); border-width: 0px; }
.usage { width: 72px; height: 5px; border-radius: 999px; color: var(--accent); background-color: var(--surface); }
.usage.warn { color: var(--warning); }
.usage-label { font-size: 11px; color: var(--muted); }

.feed-pad { padding: 18px 24px 16px 24px; max-width: 920px; }
.user-bubble { padding: 10px 14px; border-radius: 18px; background: linear-gradient(135deg, var(--accent), var(--accent-2)); max-width: 620px; box-shadow: 0 3px 10px var(--accent-dim); }
.user-text { color: var(--accent-fg); font-size: 14px; }
.assistant { max-width: 860px; }
.avatar { width: 30px; height: 30px; border-radius: 999px; background: linear-gradient(135deg, var(--accent), var(--accent-3)); justify-content: center; align-items: center; margin: 2px 0px 0px 0px; }
.avatar-ic { icon-color: #ffffff; icon-size: 18px; }
.avatar-gap { width: 30px; }
.answer { font-size: 14px; color: var(--fg); }
.actions { padding: 2px 0px 0px 0px; }
.think { padding: 8px 12px; border-radius: 12px; background-color: var(--card); border: 1px solid var(--divider); }
.think-ic { icon-size: 16px; icon-color: var(--muted); }
.think-head { font-size: 12px; color: var(--muted); }
.think-body { font-size: 12px; color: var(--muted); }
.tool-card { padding: 8px 12px; border-radius: 12px; background-color: var(--card); border: 1px solid var(--divider); max-width: 760px; }
.tool-card.failed { border-color: var(--danger); }
.tool-badge { width: 26px; height: 26px; border-radius: 8px; background-color: var(--accent-dim); justify-content: center; align-items: center; }
.tool-badge.fail { background-color: var(--danger-soft); }
.tool-ic { icon-size: 16px; icon-color: var(--accent); }
.tool-badge.fail .tool-ic { icon-color: var(--danger); }
.tool-ok { icon-size: 16px; icon-color: var(--success); }
.tool-fail { icon-size: 16px; icon-color: var(--danger); }
.tool-sum { font-size: 13px; color: var(--fg); }
.tool-out { font-size: 12px; color: var(--muted); font-family: monospace; }
.att-image { max-width: 420px; max-height: 420px; border-radius: 12px; }
.typing { accent-color: var(--accent); }
.spin { color: var(--accent); }
.error-card { padding: 10px 12px; border-radius: 12px; background-color: var(--danger-soft); border: 1px solid var(--danger); max-width: 760px; }
.error-ic { icon-color: var(--danger); }
.error { font-size: 13px; color: var(--fg); }

.confirm { margin: 0px 24px 8px 24px; padding: 12px 14px; border-radius: 14px; background-color: var(--card); border: 1px solid var(--accent); box-shadow: 0 6px 20px var(--shadow); }
.confirm.voice { margin: 0px; max-width: 560px; }
.confirm-badge { width: 36px; height: 36px; border-radius: 10px; background-color: var(--accent-dim); justify-content: center; align-items: center; }
.confirm-ic { icon-size: 22px; icon-color: var(--accent); }
.confirm-title { font-size: 13px; font-weight: 600; color: var(--fg); }
.confirm-sum { font-size: 12px; color: var(--fg); font-family: monospace; }

/* ── Ввод ─────────────────────────────────────────────────────────── */

.compose-wrap { padding: 6px 24px 16px 24px; }
.composer { padding: 8px 8px 8px 8px; border-radius: 20px; background-color: var(--field); border: 1px solid var(--border); box-shadow: 0 8px 28px var(--shadow); transition: border-color 150ms ease-out; }
.composer:focus-within { border-color: var(--accent); }
MultilineTextEdit.input { background-color: #00000000; color: var(--fg); border: 0px solid #00000000; padding: 8px 6px; font-size: 14px; caret-color: var(--accent); }
.att-row { padding: 2px 4px; }
.att-chip { font-size: 12px; background-color: var(--accent-dim); color: var(--fg); }
.dict-row { padding: 4px 8px; }
.dict-text { font-size: 12px; color: var(--muted); }
.wave { accent-color: var(--accent); }
.warn-ic { icon-size: 16px; icon-color: var(--warning); }
.warn-text { font-size: 12px; color: var(--muted); }

/* ── Пустой чат ───────────────────────────────────────────────────── */

.empty { padding: 32px 24px 24px 24px; }
.orb { accent-color: var(--accent); color: var(--accent-2); }
.hero-title { font-size: 26px; font-weight: 600; color: var(--fg); }
.hero-sub { font-size: 13px; color: var(--muted); max-width: 520px; text-align: center; }
.examples { width: 100%; max-width: 560px; padding: 8px 0px; }

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
.voice-meter { width: 300px; height: 6px; border-radius: 999px; color: var(--muted); background-color: var(--surface); }
.voice-meter.on { color: var(--success); }
.voice-meter-mark-row { width: 300px; height: 6px; }
.voice-meter-mark { width: 2px; height: 6px; background-color: var(--danger); }
.voice-meter-text { font-size: 11px; color: var(--muted); text-align: center; }
.voice-meter-state { font-size: 12px; color: var(--muted); text-align: center; }
.voice-meter-state.on { color: var(--success); font-weight: 600; }
.voice-meter-state.pause { color: var(--warning); font-weight: 600; }
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
