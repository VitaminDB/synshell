/* Пульт Курьера. Переменные (--bg, --card, --fg, --muted, --accent, --accent-fg, --link, --danger)
   задаются из темы Telegram в lib.rs (theme_mss). */

.grow { flex-grow: 1; }
Text { color: var(--fg); font-size: 15px; }
.muted { color: var(--muted); }
.small { font-size: 12px; }
.err { color: var(--danger); }
.pad { padding: 2px 4px; }

.root { background: var(--bg); }
.page { padding: 12px 12px 16px 12px; }

.card { background: var(--card); border-radius: 16px; padding: 12px; }
.card.head { padding: 14px 12px 12px 14px; }
.section { font-size: 13px; font-weight: 600; color: var(--link); padding: 0px 2px 2px 2px; }

.brand { width: 40px; height: 40px; border-radius: 20px; background: linear-gradient(135deg, var(--accent), var(--link)); justify-content: center; align-items: center; }
.brand-ic { icon-size: 22px; icon-color: var(--accent-fg); }
.title { font-size: 18px; font-weight: 600; }

.dot { width: 10px; height: 10px; border-radius: 5px; background-color: var(--muted); }
.dot.ok { background-color: #31c45a; }
.dot.busy { background-color: #f5a524; box-shadow: 0 0 8px #f5a524; }
.dot.off { background-color: #00000000; border: 1px solid var(--muted); }

.chip { padding: 3px 10px; border-radius: 999px; border: 1px solid var(--muted); }
.chip.on { border-color: var(--accent); background-color: var(--accent); }
.chip-t { font-size: 12px; color: var(--muted); }
.chip.on .chip-t { color: var(--accent-fg); }

.btn { padding: 10px 14px; border-radius: 14px; background-color: var(--card); color: var(--fg); icon-color: var(--fg); font-size: 14px; transition: background-color 120ms ease-out; }
.btn:pressed { background-color: var(--muted); }
.btn.flat { background-color: #00000000; icon-color: var(--muted); }
.btn.round { padding: 8px; border-radius: 999px; }
.btn.primary { background-color: var(--accent); icon-color: var(--accent-fg); color: var(--accent-fg); }
.btn.send { padding: 12px; }
.btn.small-btn { padding: 4px; icon-size: 16px; }

.tile-wrap { min-width: 0px; }
.btn.tile { flex-direction: column; padding: 14px 6px; border-radius: 16px; font-size: 12px; icon-size: 24px; icon-color: var(--accent); gap: 6px; text-align: center; }
.btn.tile.danger { background-color: var(--danger); color: #ffffff; icon-color: #ffffff; }

.s-row { padding: 10px 8px; border-radius: 12px; background-color: #00000000; }
.s-row:pressed { background-color: var(--bg); }
.s-row.on { background-color: var(--bg); border: 1px solid var(--accent); }
.s-name { font-size: 15px; }
.ic-sm { icon-size: 13px; icon-color: var(--muted); }

.msg { padding: 8px 10px; border-radius: 12px; max-width: 92%; }
.msg.in { background-color: var(--accent); align-self: flex-end; }
.msg.in .msg-t { color: var(--accent-fg); }
.msg.out { background-color: var(--bg); align-self: flex-start; }
.msg-t { font-size: 13px; }

.composer { background: var(--card); padding: 8px 10px 10px 10px; border-top: 1px solid var(--bg); }
.to { font-size: 12px; color: var(--link); }
.field { background-color: var(--bg); border-radius: 20px; padding: 10px 14px; color: var(--fg); font-size: 15px; }

.toast { margin: 0px 12px 8px 12px; padding: 10px 14px; border-radius: 12px; background-color: #000000c0; align-self: center; }
.toast-t { color: #ffffff; font-size: 13px; }
