/* syn-remote — «Пульт». Палитра (--bg, --fg, --accent, …) — из [appearance], свои переменные — в main.rs. */

.grow { flex-grow: 1; }
.ic { icon-size: 20px; icon-color: var(--muted); }
Text { color: var(--fg); font-size: 14px; }
.hint { font-size: 12px; color: var(--muted); }
.small { font-size: 11px; }
.center { text-align: center; }
.muted { color: var(--muted); }
.title { font-size: 19px; font-weight: 600; color: var(--fg); }
.section { font-size: 12px; font-weight: 600; color: var(--muted); padding: 4px 2px 0px 2px; }
.page { padding: 16px; }
.narrow-page { max-width: 640px; }

/* ── Окно ─────────────────────────────────────────────────────────── */

.window { background: var(--content); color: var(--fg); }
.window.phone { background: var(--bg); }
.window-frame { border-radius: var(--window-radius); border: 1px solid var(--border); }
.window-frame.flat { border-radius: 0px; border-width: 0px; }
.window-frame:window-maximized, .window-frame:window-fullscreen { border-radius: 0px; border-width: 0px; }
.body { background: var(--content); }
.window.phone .body { background: var(--bg); }

.titlebar { height: 46px; background: var(--titlebar); }
.tb-left { height: 46px; width: 248px; padding: 0px 10px 0px 12px; align-items: center; }
.drag-wrap { height: 46px; }
.drag-space { height: 46px; flex-grow: 1; min-width: 40px; align-items: center; justify-content: center; }
.tb-title-row { height: 46px; }
.tb-title { font-size: 13px; font-weight: 600; color: var(--fg); max-width: 520px; }
.tb-tools { height: 46px; padding: 0px 6px; align-items: center; }
.window-controls { height: 46px; padding: 0px 4px; align-items: center; }
SystemWindowControls { color: var(--fg); background-color: var(--hover); }

.brand { width: 28px; height: 28px; border-radius: 9px; background: linear-gradient(135deg, var(--accent), var(--accent-2)); justify-content: center; align-items: center; }
.brand-ic { icon-color: #ffffff; icon-size: 18px; }
.brand-title { font-size: 14px; font-weight: 600; color: var(--fg); }

.appbar { height: 56px; padding: 4px 6px 4px 6px; background: var(--titlebar); }
.appbar-mark { padding: 0px 6px 0px 8px; }
.appbar-drag { height: 48px; justify-content: center; align-items: center; padding: 0px 6px; }
.appbar-title { font-size: 17px; font-weight: 600; color: var(--fg); }

/* светодиод передатчика */
.led { width: 10px; height: 10px; border-radius: 5px; background-color: var(--surface-alt); border: 1px solid var(--border); margin: 0px 8px; transition: background-color 90ms ease-out; }
.led.on { background-color: #ff4d4d; border-color: #ff8080; box-shadow: 0 0 12px #ff3030; }
.led.off { background-color: #00000000; border-color: var(--muted); }

.fs-bar { padding: 6px 10px; }
.btn.fs-btn { padding: 8px; background-color: #00000000; icon-color: var(--muted); }
.btn.fs-btn:hover { background-color: var(--hover); icon-color: var(--fg); }

/* ── Боковая панель ───────────────────────────────────────────────── */

.sidebar { width: 260px; padding: 12px 10px 10px 10px; background: var(--sidebar); border-right: 1px solid var(--border); }
.side-section { padding: 10px 6px 2px 6px; }
.side-empty { padding: 6px; }
.side-item { padding: 9px 10px; border-radius: 10px; transition: background-color 120ms ease-out; }
.side-item:hover { background-color: var(--hover); }
.side-item.on { background-color: var(--accent-dim); }
.side-ic { icon-size: 20px; icon-color: var(--muted); }
.side-item.on .side-ic { icon-color: var(--accent); }
.side-title { font-size: 13px; color: var(--fg); }

/* ── Кнопки ───────────────────────────────────────────────────────── */

.btn { padding: 9px 14px; border-radius: 12px; background-color: var(--surface); color: var(--fg); font-size: 13px; icon-color: var(--fg); transition: background-color 120ms ease-out; }
.btn:hover { background-color: var(--hover); }
.btn:pressed { background-color: var(--pressed); }
.btn.primary { background: linear-gradient(135deg, var(--accent), var(--accent-2)); color: var(--accent-fg); icon-color: var(--accent-fg); }
.btn.primary:hover { background: linear-gradient(135deg, var(--accent-2), var(--accent)); }
.btn.primary:disabled { background: var(--surface-alt); color: var(--muted); icon-color: var(--muted); }
.btn.big { padding: 13px 20px; border-radius: 16px; font-size: 15px; font-weight: 600; }
.btn.ghost { background-color: #00000000; border: 1px solid var(--border); }
.btn.ghost:hover { background-color: var(--hover); }
.btn.flat { background-color: #00000000; icon-color: var(--muted); }
.btn.flat:hover { background-color: var(--hover); }
.btn.round { padding: 8px; border-radius: 999px; }
.btn.tb-btn { padding: 8px; icon-size: 21px; icon-color: var(--muted); }
.btn.tb-btn:hover { icon-color: var(--fg); }
.btn.new { padding: 11px 12px; border-radius: 12px; background: var(--accent-dim); color: var(--fg); icon-color: var(--accent); font-weight: 600; }
.btn.new:hover { background: var(--accent-soft); }
.btn.chip { padding: 7px 12px; border-radius: 999px; font-size: 13px; background-color: var(--surface); icon-size: 16px; icon-color: var(--muted); }
.btn.chip:hover { background-color: var(--hover); }
.btn.row-test { padding: 8px; icon-color: #ff6b6b; }
.btn.key-toggle { padding: 10px; icon-size: 22px; background-color: var(--key); icon-color: var(--fg); }
.btn.key-toggle.on { background-color: var(--accent-soft); icon-color: var(--accent); }

TextField.field { background-color: var(--surface); border: 1px solid var(--border); border-radius: 12px; padding: 10px 12px; font-size: 14px; color: var(--fg); }
TextField.field:focus { border-color: var(--accent); }
Dropdown.field { background-color: var(--surface); border-radius: 12px; padding: 9px 12px; font-size: 13px; color: var(--fg); }
.bar { height: 6px; border-radius: 3px; }
.pick-bar { width: 220px; }

/* ── Карточки и списки ────────────────────────────────────────────── */

.card { padding: 16px; border-radius: 18px; background-color: var(--surface); border: 1px solid var(--border); }
.card-title { font-size: 15px; font-weight: 600; color: var(--fg); }
.card-badge { width: 40px; height: 40px; border-radius: 12px; background-color: var(--accent-dim); justify-content: center; align-items: center; }
.card-badge-ic { icon-size: 22px; icon-color: var(--accent); }
.banner { padding: 10px 12px; border-radius: 12px; background-color: var(--surface); border: 1px solid var(--warning); }
.warn-ic { icon-color: var(--warning); }
.muted-ic { icon-size: 18px; }

.empty { padding: 48px 20px 24px 20px; }
.hero { width: 96px; height: 96px; border-radius: 48px; background-color: var(--accent-dim); justify-content: center; align-items: center; }
.hero-ic { icon-size: 48px; icon-color: var(--accent); }
.hero.ok { width: 72px; height: 72px; border-radius: 36px; background-color: var(--accent-soft); }
.hero.ok .hero-ic { icon-size: 36px; }

.remote-card { padding: 14px; border-radius: 18px; background-color: var(--surface); border: 1px solid var(--border); min-height: 130px; transition: background-color 120ms ease-out; }
.remote-card:hover { background-color: var(--hover); }
.remote-card.add { background-color: #00000000; border: 1px dashed var(--border); align-items: center; justify-content: center; }
.add-ic { icon-size: 30px; icon-color: var(--muted); }

.tile { padding: 14px; border-radius: 16px; background-color: var(--surface); border: 1px solid var(--border); height: 132px; transition: background-color 120ms ease-out; }
.tile:hover { background-color: var(--hover); }

.row-item { padding: 10px 12px; border-radius: 14px; background-color: var(--surface); transition: background-color 120ms ease-out; }
.row-item:hover { background-color: var(--hover); }
.row-item.accent { background-color: var(--accent-dim); border: 1px solid var(--accent-soft); }
.row-hit { padding: 6px 0px; }
.row-title { font-size: 14px; color: var(--fg); }
.row-badge { width: 34px; height: 34px; border-radius: 10px; background-color: var(--surface-alt); justify-content: center; align-items: center; }
.row-badge.on { background: linear-gradient(135deg, var(--accent), var(--accent-2)); }
.row-badge-ic { icon-size: 19px; icon-color: var(--accent); }
.row-badge.on .row-badge-ic { icon-color: #ffffff; }
.chevron { icon-size: 20px; }

.stat { padding: 10px 14px; border-radius: 12px; background-color: var(--surface-alt); flex-grow: 1; }
.stat-value { font-size: 18px; font-weight: 700; color: var(--fg); }

/* ── Подбор ───────────────────────────────────────────────────────── */

.send-ring { width: 168px; height: 168px; border-radius: 84px; padding: 16px; background-color: var(--accent-dim); border: 2px solid var(--accent-soft); transition: background-color 150ms ease-out; }
.send-ring.lit { background-color: var(--accent-soft); border-color: var(--accent); box-shadow: 0 0 36px var(--accent-glow); }
.send-core { width: 132px; height: 132px; border-radius: 66px; background: linear-gradient(135deg, var(--accent), var(--accent-2)); justify-content: center; align-items: center; box-shadow: 0 10px 28px var(--accent-glow); }
.send-ic { icon-size: 58px; icon-color: #ffffff; }
.tests { padding: 2px; border-radius: 14px; background-color: var(--surface); }
.btn.test { padding: 8px 12px; border-radius: 12px; background-color: #00000000; color: var(--muted); icon-color: var(--muted); font-size: 13px; }
.btn.test:hover { background-color: var(--hover); }
.btn.test.on { background-color: var(--accent-soft); color: var(--fg); icon-color: var(--accent); }

/* ── Пульт ────────────────────────────────────────────────────────── */

.remote-head { padding: 0px 2px; }
.remote-body { padding: 22px 18px 26px 18px; border-radius: 34px; background: var(--remote-body); border: 1px solid var(--border); box-shadow: 0 14px 40px var(--shadow); }
.board-row { width: 100%; }

.key { background-color: var(--key); border-radius: 16px; justify-content: center; align-items: center; transition: background-color 90ms ease-out, scale 90ms ease-out; }
.key:hover { background-color: var(--key-hi); }
.key.lit { background-color: var(--accent-soft); scale: 0.94; }
.key.off { background-color: #00000000; border: 1px dashed var(--border); }
.key.off .key-ic { icon-color: var(--border); }
.key.off .key-text { color: var(--border); }
.key-ic { icon-size: 24px; icon-color: var(--fg); }
.key-text { font-size: 13px; font-weight: 600; color: var(--fg); }

.key.round { width: 56px; height: 56px; border-radius: 28px; }
.key.round.small { width: 50px; height: 50px; border-radius: 25px; }
.key.round.small .key-ic { icon-size: 22px; }
.key.round.media { width: 44px; height: 44px; border-radius: 22px; }
.key.round.media .key-ic { icon-size: 21px; }
.key.power { background-color: #e5484d; }
.key.power .key-ic { icon-color: #ffffff; icon-size: 26px; }
.key.power:hover { background-color: #f05a5f; }
.key.power.lit { background-color: #ff7a7e; }
.key.pill { height: 40px; padding: 0px 16px; border-radius: 20px; }
.key.power-soft .key-text { color: #ff6b6b; }

.dpad { width: 228px; height: 228px; border-radius: 114px; background-color: var(--key); padding: 8px; justify-content: center; align-items: center; }
.dpad-gap { width: 64px; height: 64px; }
.key.dpad-key { width: 64px; height: 64px; border-radius: 32px; background-color: #00000000; }
.key.dpad-key:hover { background-color: var(--key-hi); }
.key.dpad-key.lit { background-color: var(--accent-soft); }
.key.dpad-key .key-ic { icon-size: 34px; }
.key.dpad-ok { width: 80px; height: 80px; border-radius: 40px; background-color: var(--key-hi); border: 1px solid var(--border); }
.key.dpad-ok.lit { background-color: var(--accent-soft); }
.key.dpad-ok .key-text { font-size: 16px; }

.rocker { width: 62px; padding: 4px 0px; border-radius: 31px; background-color: var(--key); }
.key.rock { width: 58px; height: 58px; border-radius: 29px; background-color: #00000000; }
.key.rock:hover { background-color: var(--key-hi); }
.key.rock.lit { background-color: var(--accent-soft); }
.key.rock.off { border-width: 0px; }
.rock-label { font-size: 10px; font-weight: 700; color: var(--muted); padding: 6px 0px; }

.numpad { padding: 4px 8px; }
.key.num { width: 72px; height: 50px; border-radius: 14px; }
.key.num .key-text { font-size: 18px; font-weight: 500; }
.num-gap { width: 72px; height: 50px; }

.key.color { width: 54px; height: 22px; border-radius: 11px; }
.key.color.red { background-color: #e5484d; }
.key.color.green { background-color: #30a46c; }
.key.color.yellow { background-color: #f5d90a; }
.key.color.blue { background-color: #3e63dd; }
.key.color.lit { scale: 0.9; }
.key.app { background-color: var(--key); }
.key.app.netflix .key-text { color: #e50914; }
.key.app.youtube .key-text { color: #ff3b30; }
.key.app.prime .key-text { color: #00a8e1; }

.key.cell { width: 100%; height: 48px; padding: 0px 8px; border-radius: 14px; }
.key.cell .key-text { font-size: 12px; font-weight: 500; }

/* ── Меню, диалоги, сообщения ─────────────────────────────────────── */

.scrim-clear { width: 100%; height: 100%; background-color: #00000001; }
.scrim { width: 100%; height: 100%; background-color: var(--scrim); }
.menu-wrap { padding: 104px 14px 0px 14px; }
.menu { width: 280px; padding: 6px; border-radius: 16px; background-color: var(--menu-bg); border: 1px solid var(--border); box-shadow: 0 12px 32px var(--shadow); }
.menu-item { padding: 10px 12px; border-radius: 10px; }
.menu-item:hover { background-color: var(--hover); }
.menu-section { padding: 8px 12px 2px 12px; }
.menu-ic { icon-size: 20px; }
.menu-text { font-size: 14px; color: var(--fg); }
.menu-item.danger .menu-ic { icon-color: var(--danger); }
.menu-item.danger .menu-text { color: var(--danger); }
.dialog-wrap { width: 100%; height: 100%; padding: 24px; }
.dialog-card { width: 340px; padding: 20px; border-radius: 20px; background-color: var(--menu-bg); border: 1px solid var(--border); box-shadow: 0 16px 40px var(--shadow); }

.toast-wrap { width: 100%; height: 100%; padding: 0px 16px 36px 16px; }
.toast { padding: 11px 18px; border-radius: 14px; background-color: var(--menu-bg); border: 1px solid var(--border); box-shadow: 0 8px 24px var(--shadow); max-width: 420px; }
.toast-text { font-size: 13px; color: var(--fg); }
