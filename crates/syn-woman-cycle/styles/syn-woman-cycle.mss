/* syn-woman-cycle — «Цикл». Переменные палитры (светлая/тёмная) — в main.rs.
   Стиль: спокойный тёплый фон, белые карточки с тонкой рамкой и мягкой тенью, сплошные
   насыщенные цвета фаз без глянца и лишних градиентов, крупная типографика. */

.grow { flex-grow: 1; }
.ic { icon-size: 20px; icon-color: var(--muted); }
Text { color: var(--fg); font-size: 14px; }
.hint { font-size: 13px; color: var(--muted); line-height: 18px; }
.center { text-align: center; }
.h1 { font-size: 30px; font-weight: 800; color: var(--fg); letter-spacing: -0.8px; }
.h2 { font-size: 20px; font-weight: 700; color: var(--fg); letter-spacing: -0.3px; }
.over { font-size: 13px; font-weight: 500; color: var(--muted); }
.section { font-size: 12px; font-weight: 700; color: var(--muted); letter-spacing: 0.9px; text-transform: uppercase; padding: 8px 4px 0px 4px; }
.disclaimer { padding: 8px 12px 20px 12px; }
.row-title { font-size: 15px; font-weight: 600; color: var(--fg); }
.chevron { icon-size: 22px; icon-color: var(--faint); }
.tiny-ic { icon-size: 16px; icon-color: var(--faint); }

/* ── Окно ─────────────────────────────────────────────────────────── */

.window { background-color: #00000000; color: var(--fg); }
.window-frame { background-color: var(--bg); border-radius: var(--window-radius); border: 1px solid var(--border); }
.window-frame.flat { border-radius: 0px; border-width: 0px; }
.window-frame:window-maximized, .window-frame:window-fullscreen { border-radius: 0px; border-width: 0px; }
.body { background-color: #00000000; }
.ios-root { background-color: var(--bg); }

.titlebar { height: 44px; background-color: var(--titlebar); border-bottom: 1px solid var(--border); }
.tb-left { height: 44px; width: 250px; padding: 0px 10px 0px 14px; align-items: center; }
.drag-wrap { height: 44px; }
.drag-space { height: 44px; flex-grow: 1; min-width: 40px; }
.tb-tools { height: 44px; padding: 0px 4px; align-items: center; }
.window-controls { height: 44px; padding: 0px 4px; align-items: center; }
SystemWindowControls { color: var(--fg); background-color: var(--hover); }

.brand { width: 28px; height: 28px; border-radius: 9px; background-color: var(--rose); justify-content: center; align-items: center; }
.brand-ic { icon-color: #ffffff; icon-size: 17px; }
.brand-title { font-size: 15px; font-weight: 700; color: var(--fg); }

/* ── Боковая панель и нижняя навигация ────────────────────────────── */

.sidebar { width: 250px; padding: 16px 12px 16px 12px; background-color: var(--sidebar); border-right: 1px solid var(--border); }
.side-item { padding: 12px 14px; border-radius: 12px; transition: background-color 180ms ease-out; }
.side-item:hover { background-color: var(--hover); }
.side-item.on { background-color: var(--surface); box-shadow: 0px 1px 3px var(--shadow); }
.side-ic { icon-size: 22px; icon-color: var(--muted); }
.side-item.on .side-ic { icon-color: var(--rose); }
.side-title { font-size: 14px; font-weight: 600; color: var(--fg); }
.side-note { max-width: 190px; }

.bottom-nav { padding: 8px 2px 10px 2px; background-color: var(--nav); border-top: 1px solid var(--border); }
.nav-item { padding: 2px 0px; }
.nav-pill { width: 44px; height: 30px; border-radius: 15px; justify-content: center; align-items: center; background-color: #00000000; transition: width 320ms spring(380, 26), background-color 220ms ease-out; }
.nav-pill.on { width: 60px; background-color: var(--rose-soft); }
.nav-ic { icon-size: 23px; icon-color: var(--muted); transition: icon-color 200ms ease-out; }
.nav-ic.on { icon-color: var(--rose); }
.nav-label { font-size: 11px; font-weight: 500; color: var(--muted); }
.nav-label.on { color: var(--fg); font-weight: 700; }

/* ── Страница ─────────────────────────────────────────────────────── */

.page { padding: 8px 16px 24px 16px; }
.page.wide { padding: 20px 16px 32px 16px; }
.page-header { padding: 10px 0px 6px 2px; }
.header-drag { padding: 4px 0px; }

.hero { padding: 22px 18px 18px 18px; border-radius: 24px; background-color: var(--surface); border: 1px solid var(--border); box-shadow: 0px 6px 20px var(--shadow); align-items: center; }
.ring-over { font-size: 13px; font-weight: 700; color: var(--muted); text-transform: uppercase; letter-spacing: 0.8px; text-align: center; }
.ring-big { font-size: 44px; font-weight: 800; color: var(--fg); letter-spacing: -1px; text-align: center; }
.ring-sub { font-size: 14px; color: var(--muted); text-align: center; }

/* плашки со знаком: мягкий тон фазы + знак её цвета */
.tile { padding: 14px; border-radius: 20px; background-color: var(--surface); border: 1px solid var(--border); }
.tile-label { font-size: 12px; font-weight: 600; color: var(--muted); }
.tile-value { font-size: 18px; font-weight: 700; color: var(--fg); }
.tile-badge { width: 38px; height: 38px; border-radius: 12px; justify-content: center; align-items: center; background-color: var(--surface-alt); }
.tile-ic { icon-size: 20px; icon-color: var(--muted); }
.tile-badge.rose { background-color: var(--rose-soft); }
.tile-badge.rose .tile-ic { icon-color: var(--rose); }
.tile-badge.rose-solid { background-color: var(--rose); }
.tile-badge.rose-solid .tile-ic { icon-color: #ffffff; }
.tile-badge.rose-o { background-color: #00000000; border: 1.5px dashed var(--rose); }
.tile-badge.rose-o .tile-ic { icon-color: var(--rose); }
.tile-badge.teal { background-color: var(--teal-soft); }
.tile-badge.teal .tile-ic { icon-color: var(--teal); }
.tile-badge.plum { background-color: var(--plum-soft); }
.tile-badge.plum .tile-ic { icon-color: var(--plum); }
.tile-badge.gold, .tile-badge.fert { background-color: var(--fert); }
.tile-badge.gold .tile-ic, .tile-badge.fert .tile-ic { icon-color: var(--fert-text); }
.tile-badge.love { background-color: var(--love); }
.tile-badge.love .tile-ic { icon-color: #ffffff; }
.event-when { font-size: 13px; font-weight: 600; color: var(--muted); }

.row-card { padding: 14px; border-radius: 20px; background-color: var(--surface); border: 1px solid var(--border); transition: background-color 160ms ease-out, scale 140ms ease-out; }
.row-card:hover { background-color: var(--surface-alt); }
.row-card:active { scale: 0.98; }

/* ── Кнопки ───────────────────────────────────────────────────────── */

.btn { padding: 11px 16px; border-radius: 14px; background-color: var(--surface); border: 1px solid var(--border); color: var(--fg); font-size: 14px; font-weight: 600; icon-color: var(--fg); icon-size: 19px; transition: background-color 160ms ease-out, scale 140ms ease-out; }
.btn:hover { background-color: var(--surface-alt); }
.btn:active { scale: 0.97; }
.btn.big { padding: 16px 20px; border-radius: 16px; font-size: 16px; }
.btn.primary { background-color: var(--primary); border-width: 0px; color: var(--on-primary); icon-color: var(--on-primary); }
.btn.primary:hover { background-color: var(--primary); opacity: 0.92; }
.btn.primary:disabled { background-color: var(--surface-alt); color: var(--faint); icon-color: var(--faint); }
.btn.soft { background-color: var(--rose-soft); border-width: 0px; color: var(--rose); icon-color: var(--rose); }
.btn.soft:hover { background-color: var(--rose); color: #ffffff; icon-color: #ffffff; }
.btn.ghost { background-color: #00000000; border: 1px solid var(--border); color: var(--muted); icon-color: var(--muted); }
.btn.ghost:hover { background-color: var(--hover); }
.btn.flat { background-color: #00000000; border-width: 0px; icon-color: var(--muted); }
.btn.flat:hover { background-color: var(--hover); }
.btn.round { padding: 9px; border-radius: 999px; }
.btn.tb-btn { padding: 9px; icon-size: 22px; icon-color: var(--muted); }
.btn.nav-btn { padding: 8px; icon-size: 24px; icon-color: var(--fg); }
.btn.chip { padding: 8px 14px; border-radius: 999px; font-size: 13px; font-weight: 600; background-color: var(--surface); border: 1px solid var(--border); color: var(--fg); }
.btn.danger { background-color: #00000000; border: 1px solid var(--border); color: var(--danger); icon-color: var(--danger); }
.btn.danger-solid { background-color: var(--danger); border-width: 0px; color: #ffffff; }
.btn.step { padding: 8px; background-color: var(--surface-alt); border-width: 0px; icon-color: var(--fg); icon-size: 18px; }
.btn.step:disabled { icon-color: var(--faint); }

.btn.pick { padding: 8px 13px; border-radius: 999px; font-size: 13px; font-weight: 500; background-color: var(--surface); color: var(--fg); icon-size: 17px; icon-color: var(--muted); border: 1px solid var(--border); }
.btn.pick.rose.on { background-color: var(--rose); border-color: var(--rose); color: #ffffff; icon-color: #ffffff; }
.btn.pick.plum.on { background-color: var(--plum); border-color: var(--plum); color: #ffffff; icon-color: #ffffff; }
.btn.pick.gold.on { background-color: var(--ovul); border-color: var(--ovul); color: #ffffff; icon-color: #ffffff; }

Toggle { width: 48px; height: 28px; background: var(--pressed); color: #ffffff; accent-color: var(--teal); border-radius: 14px; transition: background-color 200ms ease-out; }
Toggle:checked { background: var(--teal); }
Slider { height: 6px; background: var(--pressed); color: var(--fg); accent-color: var(--fg); border-radius: 3px; }

/* ── Календарь: шапка ─────────────────────────────────────────────── */

.cal-head-wrap { padding: 10px 16px 0px 16px; }
.head-card { padding: 18px 20px 20px 20px; border-radius: 26px; background-color: var(--surface); border: 1px solid var(--border); box-shadow: 0px 8px 28px var(--shadow); }
.hd-over { font-size: 14px; font-weight: 600; color: var(--muted); }
.hd-title { font-size: 38px; font-weight: 800; color: var(--fg); letter-spacing: -1.3px; }
.hd-of { font-size: 18px; font-weight: 600; color: var(--faint); padding: 0px 0px 6px 0px; }

.ph-pill { padding: 7px 10px 7px 12px; border-radius: 999px; background-color: var(--late); }
.ph-pill.period { background-color: var(--rose); }
.ph-pill.folli { background-color: var(--folli); }
.ph-pill.fert { background-color: var(--fert); }
.ph-pill.lute { background-color: var(--lute); }
.ph-pill-text { font-size: 14px; font-weight: 700; color: #ffffff; }
.ph-pill-ic { icon-size: 18px; icon-color: #ffffff; }
.ph-pill .ph-dot { background-color: #ffffff; }

.kv-label { font-size: 13px; font-weight: 500; color: var(--muted); }
.kv-value { font-size: 13px; font-weight: 700; color: var(--fg); }

.ph-dot { width: 10px; height: 10px; border-radius: 5px; background-color: var(--faint); }
.ph-dot.period { background-color: var(--rose); }
.ph-dot.folli { background-color: var(--folli); }
.ph-dot.fert { background-color: var(--ovul); }
.ph-dot.lute { background-color: var(--lute); }
.ph-dot.late { background-color: var(--late); }
.ph-tick { font-size: 11px; font-weight: 600; color: var(--faint); }

.ph-chip { padding: 9px 7px 9px 8px; border-radius: 14px; background-color: var(--late); border: 2px solid #00000000; transition: scale 140ms ease-out, border-color 180ms ease-out; }
.ph-chip:active { scale: 0.95; }
.ph-chip.period { background-color: var(--rose); }
.ph-chip.folli { background-color: var(--folli); }
.ph-chip.fert { background-color: var(--fert); }
.ph-chip.lute { background-color: var(--lute); }
.ph-chip.on { border-color: var(--fg); }
.ph-chip .ph-dot { background-color: #ffffff; }
.ph-label { font-size: 11px; font-weight: 600; color: #ffffffd9; }
.ph-label.on { color: #ffffff; font-weight: 800; }
.ph-days { font-size: 12px; font-weight: 800; color: #ffffff; }

/* ── Календарь: лента месяцев ─────────────────────────────────────── */

.cal-head { padding: 0px; }
.ribbon-inner { padding: 0px 16px 28px 16px; }
.month-head { padding: 22px 4px 10px 4px; }
.month-title { font-size: 22px; font-weight: 800; color: var(--fg); letter-spacing: -0.5px; }
.month-title.cur { color: var(--fg); }
.wd { height: 22px; align-items: center; justify-content: center; }
.wd-text { font-size: 12px; font-weight: 700; color: var(--muted); letter-spacing: 0.3px; }
.wd-text.weekend { color: var(--rose); }

.cell-slot { justify-content: center; align-items: center; }
.cell { border-radius: 999px; justify-content: center; align-items: center; background-color: #00000000; border: 2px solid #00000000; transition: background-color 200ms ease-out, border-color 200ms ease-out, scale 180ms spring(420, 24); }
.cell:hover { background-color: var(--hover); }
.cell:active { scale: 0.92; }
.cell.period { background-color: var(--rose); }
.cell.predicted { background-color: var(--rose-soft); }
.cell.fertile { background-color: var(--fert); }
.cell.ovul { background-color: var(--ovul); }
.cell.ovul.marked { border-color: var(--ovul-line); }
/* Сегодня — залитый бирюзовый кружок; отметка дня остаётся кольцом вокруг. */
.cell.today { background-color: var(--today); border: 3px solid var(--today); }
.cell.shape { background-color: #00000000; border-color: #00000000; }
.cell.today.period { border: 3px solid var(--rose); }
.cell.today.predicted { border: 3px solid var(--rose-soft); }
.cell.today.fertile { border: 3px solid var(--ovul); }
.cell.today.ovul { border: 3px solid var(--ovul-line); }
.cell.off { opacity: 0.35; }
.cell.sel { scale: 1.08; border-color: var(--fg); }
.cell-num { font-size: 16px; font-weight: 500; color: var(--fg); }
.cell-num.strong { color: #ffffff; font-weight: 700; }
.cell-num.rose { color: var(--rose-2); font-weight: 700; }
.cell-num.fert { color: var(--fert-text); font-weight: 700; }
.cell.ovul .cell-num.fert { color: #ffffff; }
.cell-num.teal { color: var(--teal); font-weight: 700; }
.cell-num.today { color: #ffffff; font-weight: 800; }
.cell-num.off { color: var(--faint); }
.cell-num.on-mark-dark { color: #3b2a12; font-weight: 700; }
.cell-num.on-mark-light { color: #ffffff; font-weight: 700; }

.legend { padding: 22px 16px 16px 16px; margin: 18px 0px 0px 0px; border-radius: 20px; background-color: var(--surface); border: 1px solid var(--border); }
.legend-dot { width: 14px; height: 14px; border-radius: 7px; }
.legend-dot.period { background-color: var(--rose); }
.legend-dot.predicted { background-color: var(--rose-soft); }
.legend-dot.fertile { background-color: var(--fert); }
.legend-dot.ovul { background-color: var(--ovul); }
.legend-dot.today { background-color: var(--today); }
.legend-text { font-size: 13px; color: var(--muted); }

/* оформление: карточки выбора стиля */
.decor-card { border-radius: 16px; border: 2px solid var(--border); background-color: var(--surface); transition: border-color 200ms ease-out, scale 160ms spring(420, 26); }
.decor-card:active { scale: 0.96; }
.decor-card.on { border-color: var(--fg); }
.decor-art { border-radius: 14px 14px 0px 0px; }
.decor-name { padding: 6px 4px 8px 4px; align-items: center; }
.decor-name-text { font-size: 12px; font-weight: 600; color: var(--muted); text-align: center; }
.decor-card.on .decor-name-text { color: var(--fg); font-weight: 800; }

/* меню отметок у даты */
.day-menu { padding: 6px; border-radius: 18px; background-color: var(--menu-bg); border: 1px solid var(--border); box-shadow: 0px 16px 40px var(--shadow-strong); transition: size 260ms spring(420, 34); }
.menu-head { padding: 8px 10px 8px 10px; }
.menu-title { font-size: 17px; font-weight: 800; color: var(--fg); }
.menu-sep { height: 1px; margin: 4px 8px; background-color: var(--border); }
.mi { padding: 9px 10px; border-radius: 12px; transition: background-color 140ms ease-out; }
.mi:hover { background-color: var(--hover); }
.mi:active { background-color: var(--pressed); }
.mi-badge { width: 32px; height: 32px; border-radius: 10px; justify-content: center; align-items: center; background-color: var(--surface-alt); }
.mi-badge.rose { background-color: var(--rose); }
.mi-badge.fert { background-color: var(--ovul); }
.mi-badge.ink { background-color: var(--love); }
.mi-badge.rose .mi-ic, .mi-badge.fert .mi-ic, .mi-badge.ink .mi-ic { icon-color: #ffffff; }
.mi-ic { icon-size: 18px; icon-color: var(--muted); }
.mi-text { font-size: 15px; color: var(--fg); }
.mi-text.off { color: var(--faint); }
.mi-check { icon-size: 20px; icon-color: var(--teal); }

/* ── Листы: карточка дня и описание фазы ──────────────────────────── */

.scrim { background-color: var(--scrim); }
.sheet { border-radius: 28px 28px 0px 0px; background-color: var(--menu-bg); box-shadow: 0px -10px 40px var(--shadow-strong); }
.sheet.wide { border-radius: 28px; border: 1px solid var(--border); }
.sheet-inner { padding: 10px 20px 24px 20px; }
.sheet-scroll { height: fit-content; }
.grabber { width: 40px; height: 5px; border-radius: 3px; background-color: var(--border); }
.sheet-title { font-size: 26px; font-weight: 800; color: var(--fg); letter-spacing: -0.6px; }
.status { padding: 10px 14px; border-radius: 14px; background-color: var(--surface-alt); }
.status.rose { background-color: var(--rose-soft); }
.status.rose-o { background-color: var(--rose-soft); }
.status.teal { background-color: var(--fert); }
.status-text { font-size: 14px; font-weight: 600; color: var(--fg); }
.period-row { padding: 12px 14px; border-radius: 18px; background-color: var(--surface-alt); border: 1px solid #00000000; transition: background-color 220ms ease-out, border-color 220ms ease-out; }
.period-row.on { background-color: var(--rose-soft); border-color: var(--rose); }

.pi-badge { width: 52px; height: 52px; border-radius: 16px; justify-content: center; align-items: center; background-color: var(--late); }
.pi-badge.period { background-color: var(--rose); }
.pi-badge.folli { background-color: var(--folli); }
.pi-badge.fert { background-color: var(--ovul); }
.pi-badge.lute { background-color: var(--lute); }
.pi-badge-ic { icon-size: 28px; icon-color: #ffffff; }
.pi-title { font-size: 24px; font-weight: 800; color: var(--fg); letter-spacing: -0.5px; }
.pi-sub { font-size: 14px; font-weight: 500; color: var(--muted); }
.pi-here { padding: 6px 12px; border-radius: 999px; background-color: var(--late-soft); }
.pi-here.period { background-color: var(--rose-soft); }
.pi-here.folli { background-color: var(--folli-soft); }
.pi-here.fert { background-color: var(--fert); }
.pi-here.lute { background-color: var(--lute-soft); }
.pi-here-text { font-size: 13px; font-weight: 700; color: var(--fg); }

.pi-tab { padding: 10px 2px 9px 2px; border-radius: 14px; background-color: var(--surface-alt); align-items: center; transition: background-color 200ms ease-out; }
.pi-tab.on.period { background-color: var(--rose); }
.pi-tab.on.folli { background-color: var(--folli); }
.pi-tab.on.fert { background-color: var(--fert); }
.pi-tab.on.lute { background-color: var(--lute); }
.pi-tab.on .pi-tab-dot { background-color: #ffffff; }
.pi-tab-dot { width: 8px; height: 8px; border-radius: 4px; }
.pi-tab-dot.period { background-color: var(--rose); }
.pi-tab-dot.folli { background-color: var(--folli); }
.pi-tab-dot.fert { background-color: var(--ovul); }
.pi-tab-dot.lute { background-color: var(--lute); }
.pi-tab-text { font-size: 10.5px; font-weight: 600; color: var(--muted); }
.pi-tab-text.on { color: #ffffff; font-weight: 800; }

.pi-fact { padding: 12px; border-radius: 16px; background-color: var(--surface-alt); }
.pi-fact-label { font-size: 11px; font-weight: 700; color: var(--muted); letter-spacing: 0.5px; text-transform: uppercase; }
.pi-fact-value { font-size: 15px; font-weight: 700; color: var(--fg); }
.pi-meter { width: 18px; height: 5px; border-radius: 3px; background-color: var(--border); }
.pi-meter.on.period { background-color: var(--rose); }
.pi-meter.on.folli { background-color: var(--folli); }
.pi-meter.on.fert { background-color: var(--ovul); }
.pi-meter.on.lute { background-color: var(--lute); }

.pi-section { font-size: 17px; font-weight: 800; color: var(--fg); letter-spacing: -0.2px; }
.pi-text { font-size: 15px; color: var(--fg); line-height: 22px; }
.pi-dot-wrap { padding: 8px 0px 0px 2px; }
.pi-dot { width: 7px; height: 7px; border-radius: 4px; background-color: var(--late); }
.pi-dot.period { background-color: var(--rose); }
.pi-dot.folli { background-color: var(--folli); }
.pi-dot.fert { background-color: var(--ovul); }
.pi-dot.lute { background-color: var(--lute); }
.pi-bullet-ic { icon-size: 18px; icon-color: var(--late); }
.pi-bullet-ic.period { icon-color: var(--rose); }
.pi-bullet-ic.folli { icon-color: var(--folli); }
.pi-bullet-ic.fert { icon-color: var(--ovul-line); }
.pi-bullet-ic.lute { icon-color: var(--lute); }

/* ── Статистика ───────────────────────────────────────────────────── */

.stat { padding: 16px; border-radius: 20px; background-color: var(--surface); border: 1px solid var(--border); }
.stat-value { font-size: 32px; font-weight: 800; color: var(--fg); letter-spacing: -1px; }
.stat-unit { font-size: 14px; font-weight: 600; color: var(--muted); padding: 0px 0px 6px 0px; }
.stat.plum .stat-value { color: var(--lute); }
.stat.rose .stat-value { color: var(--rose); }
.stat.teal .stat-value { color: var(--teal); }
.stat.gold .stat-value { color: var(--gold); }
.cycle-row { padding: 14px 16px 16px 16px; border-radius: 20px; background-color: var(--surface); border: 1px solid var(--border); }

.empty { padding: 36px 18px; border-radius: 24px; background-color: var(--surface); border: 1px solid var(--border); }
.hero-badge { width: 76px; height: 76px; border-radius: 24px; background-color: var(--rose); justify-content: center; align-items: center; }
.hero-badge.danger { background-color: var(--danger); }
.hero-ic { icon-size: 36px; icon-color: #ffffff; }
.hero-badge.danger .hero-ic { icon-color: #ffffff; }

/* ── Настройки ────────────────────────────────────────────────────── */

.group { border-radius: 20px; background-color: var(--surface); border: 1px solid var(--border); }
.group.pad { padding: 18px; }
.sep-wrap { padding: 0px 16px 0px 62px; }
.sep { height: 1px; background-color: var(--border); }
.set-row { padding: 14px 16px; }
.set-badge { width: 32px; height: 32px; border-radius: 9px; background-color: var(--late); justify-content: center; align-items: center; }
.set-badge.rose { background-color: var(--rose); }
.set-badge.blue { background-color: var(--folli); }
.set-badge.amber { background-color: var(--ovul); }
.set-badge.violet { background-color: var(--lute); }
.set-badge.pink { background-color: var(--love); }
.set-badge.teal { background-color: var(--teal); }
.set-ic { icon-size: 18px; icon-color: #ffffff; }
.step-value { width: 44px; align-items: center; }
.step-num { font-size: 19px; font-weight: 800; color: var(--fg); text-align: center; }
.step-unit { font-size: 10px; color: var(--muted); text-align: center; }
.scale-value { font-size: 18px; font-weight: 800; color: var(--fg); }
.segs { padding: 3px; border-radius: 14px; background-color: var(--surface-alt); }
.seg { padding: 9px 4px; border-radius: 11px; align-items: center; justify-content: center; background-color: #00000000; transition: background-color 240ms ease-out, box-shadow 240ms ease-out; }
.seg.on { background-color: var(--surface); box-shadow: 0px 1px 4px var(--shadow-strong); }
.seg-ic { icon-size: 20px; icon-color: var(--muted); }
.seg-ic.on { icon-color: var(--fg); }
.seg-text { font-size: 12px; color: var(--muted); }
.seg-text.on { color: var(--fg); font-weight: 700; }

.dialog { width: 340px; padding: 24px; border-radius: 26px; background-color: var(--menu-bg); border: 1px solid var(--border); box-shadow: 0px 18px 50px var(--shadow-strong); }

/* ── Знакомство ───────────────────────────────────────────────────── */

.intro-top { height: 24px; }
.intro-hero { width: 112px; height: 112px; border-radius: 34px; background-color: var(--rose); justify-content: center; align-items: center; }
.intro-hero-ic { icon-size: 58px; icon-color: #ffffff; }
.intro-title { font-size: 36px; font-weight: 800; color: var(--fg); letter-spacing: -1px; }
.intro-text { font-size: 16px; color: var(--muted); line-height: 22px; }
.step-dot { width: 8px; height: 8px; border-radius: 4px; background-color: var(--border); transition: width 320ms spring(380, 26), background-color 220ms ease-out; }
.step-dot.on { width: 26px; background-color: var(--fg); }
.cal-card { padding: 16px 14px; border-radius: 22px; background-color: var(--surface); border: 1px solid var(--border); }

/* ── Сообщения ────────────────────────────────────────────────────── */

.toast-wrap { width: 100%; height: 100%; padding: 0px 16px 96px 16px; }
.toast { padding: 12px 18px; border-radius: 14px; background-color: var(--fg); box-shadow: 0px 10px 28px var(--shadow-strong); max-width: 420px; }
.toast-text { font-size: 14px; color: var(--bg); }
.fill { width: 100%; }

/* ── Близость ─────────────────────────────────────────────────────── */

.love-row { padding: 12px 12px 12px 14px; border-radius: 18px; background-color: var(--surface-alt); border: 1px solid #00000000; transition: background-color 220ms ease-out, border-color 220ms ease-out; }
.love-row.on { background-color: var(--surface); border-color: var(--love); }
.love-ic-muted { icon-size: 20px; icon-color: var(--love); }
.time-num { font-size: 20px; font-weight: 800; color: var(--fg); }
.btn.love { background-color: var(--surface); color: var(--love); icon-color: var(--love); border: 1px solid var(--border); }
.btn.love:hover { background-color: var(--love); color: #ffffff; icon-color: #ffffff; }
.btn.danger-ic { icon-color: var(--faint); icon-size: 20px; }
.btn.danger-ic:hover { icon-color: var(--danger); }
.dot.love { background-color: var(--love); }
.btn.love-add { padding: 10px; background-color: var(--love); border-width: 0px; icon-color: #ffffff; icon-size: 22px; }

/* на жёлтом — тёмно-янтарный текст (и в тёмной теме) */
.ph-pill.fert .ph-pill-text, .ph-pill.fert .ph-pill-ic, .ph-chip.fert .ph-label, .ph-chip.fert .ph-days, .pi-tab.on.fert .pi-tab-text, .pi-here.fert .pi-here-text, .status.teal .status-text { color: var(--fert-text); icon-color: var(--fert-text); }
.ph-pill.fert .ph-dot, .ph-chip.fert .ph-dot, .pi-tab.on.fert .pi-tab-dot { background-color: var(--fert-text); }
