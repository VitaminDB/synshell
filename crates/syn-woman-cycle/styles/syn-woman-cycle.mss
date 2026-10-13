/* syn-woman-cycle — «Цикл». Переменные палитры (светлая/тёмная) — в main.rs. */

.grow { flex-grow: 1; }
.ic { icon-size: 20px; icon-color: var(--muted); }
Text { color: var(--fg); font-size: 14px; }
.hint { font-size: 12px; color: var(--muted); line-height: 17px; }
.center { text-align: center; }
.h1 { font-size: 28px; font-weight: 700; color: var(--fg); letter-spacing: -0.4px; }
.h2 { font-size: 19px; font-weight: 700; color: var(--fg); }
.over { font-size: 13px; color: var(--muted); }
.section { font-size: 12px; font-weight: 700; color: var(--muted); letter-spacing: 0.6px; text-transform: uppercase; padding: 6px 4px 0px 4px; }
.disclaimer { padding: 8px 12px 20px 12px; }
.row-title { font-size: 15px; font-weight: 600; color: var(--fg); }
.chevron { icon-size: 22px; icon-color: var(--faint); }
.tiny-ic { icon-size: 16px; icon-color: var(--faint); }

/* ── Окно ─────────────────────────────────────────────────────────── */

.window { background: linear-gradient(180deg, var(--bg-2), var(--bg)); color: var(--fg); }
.window-frame { border-radius: var(--window-radius); border: 1px solid var(--border); }
.window-frame.flat { border-radius: 0px; border-width: 0px; }
.window-frame:window-maximized, .window-frame:window-fullscreen { border-radius: 0px; border-width: 0px; }
.body { background-color: #00000000; }

.titlebar { height: 44px; background-color: var(--titlebar); border-bottom: 1px solid var(--border); }
.tb-left { height: 44px; width: 250px; padding: 0px 10px 0px 14px; align-items: center; }
.drag-wrap { height: 44px; }
.drag-space { height: 44px; flex-grow: 1; min-width: 40px; }
.tb-tools { height: 44px; padding: 0px 4px; align-items: center; }
.window-controls { height: 44px; padding: 0px 4px; align-items: center; }
SystemWindowControls { color: var(--fg); background-color: var(--hover); }

.brand { width: 28px; height: 28px; border-radius: 10px; background: linear-gradient(135deg, var(--rose), var(--plum)); justify-content: center; align-items: center; box-shadow: 0 4px 12px var(--shadow); }
.brand-ic { icon-color: #ffffff; icon-size: 17px; }
.brand-title { font-size: 15px; font-weight: 700; color: var(--fg); }

/* ── Боковая панель и нижняя навигация ────────────────────────────── */

.sidebar { width: 250px; padding: 16px 12px 16px 12px; background-color: var(--sidebar); border-right: 1px solid var(--border); }
.side-item { padding: 12px 14px; border-radius: 14px; transition: background-color 180ms ease-out; }
.side-item:hover { background-color: var(--hover); }
.side-item.on { background-color: var(--rose-soft); }
.side-ic { icon-size: 22px; icon-color: var(--muted); }
.side-item.on .side-ic { icon-color: var(--rose); }
.side-title { font-size: 14px; font-weight: 600; color: var(--fg); }
.side-note { max-width: 190px; }

.bottom-nav { padding: 8px 2px 10px 2px; background-color: var(--nav); border-top: 1px solid var(--border); }
.nav-item { padding: 2px 0px; }
.nav-pill { width: 40px; height: 32px; border-radius: 16px; justify-content: center; align-items: center; background-color: #00000000; transition: width 320ms spring(380, 26), background-color 220ms ease-out; }
.nav-pill.on { width: 64px; background-color: var(--rose-soft); }
.nav-ic { icon-size: 23px; icon-color: var(--muted); transition: icon-color 200ms ease-out; }
.nav-ic.on { icon-color: var(--rose); }
.nav-label { font-size: 11px; color: var(--muted); }
.nav-label.on { color: var(--fg); font-weight: 700; }

/* ── Страница ─────────────────────────────────────────────────────── */

.page { padding: 8px 16px 24px 16px; }
.page.wide { padding: 20px 16px 32px 16px; }
.page-header { padding: 6px 0px 4px 2px; }
.header-drag { padding: 4px 0px; }

.hero { padding: 22px 18px 18px 18px; border-radius: 30px; background: linear-gradient(160deg, var(--surface), var(--surface-alt)); border: 1px solid var(--border); box-shadow: 0 14px 36px var(--shadow); align-items: center; }
.ring-over { font-size: 13px; font-weight: 700; color: var(--muted); text-transform: uppercase; letter-spacing: 0.8px; text-align: center; }
.ring-over.rose { color: var(--rose); }
.ring-over.teal { color: var(--teal); }
.ring-over.plum { color: var(--plum); }
.ring-big { font-size: 44px; font-weight: 800; color: var(--fg); letter-spacing: -1px; text-align: center; }
.ring-sub { font-size: 14px; color: var(--muted); text-align: center; }
.ring-empty-ic { icon-size: 40px; icon-color: var(--rose); }

.tile { padding: 14px; border-radius: 22px; background-color: var(--surface); border: 1px solid var(--border); }
.tile-label { font-size: 12px; font-weight: 600; color: var(--muted); }
.tile-value { font-size: 18px; font-weight: 700; color: var(--fg); }
.tile-badge { width: 34px; height: 34px; border-radius: 12px; justify-content: center; align-items: center; background-color: var(--surface-alt); }
.tile-ic { icon-size: 19px; }
.tile-badge.rose { background-color: var(--rose-soft); }
.tile-badge.rose .tile-ic { icon-color: var(--rose); }
.tile-badge.rose-solid { background: linear-gradient(135deg, var(--rose), var(--rose-2)); }
.tile-badge.rose-solid .tile-ic { icon-color: #ffffff; }
.tile-badge.rose-o { background-color: #00000000; border: 2px dashed var(--rose-line); }
.tile-badge.rose-o .tile-ic { icon-color: var(--rose); }
.tile-badge.teal { background-color: var(--teal-soft); }
.tile-badge.teal .tile-ic { icon-color: var(--teal); }
.tile-badge.plum { background-color: var(--plum-soft); }
.tile-badge.plum .tile-ic { icon-color: var(--plum); }
.tile-badge.gold { background-color: #f2a93b26; }
.tile-badge.gold .tile-ic { icon-color: var(--gold); }

.tip { padding: 16px; border-radius: 22px; background: linear-gradient(135deg, var(--plum-soft), var(--rose-soft)); }
.tip-badge { width: 40px; height: 40px; border-radius: 20px; background-color: var(--surface); justify-content: center; align-items: center; }
.tip-ic { icon-size: 21px; icon-color: var(--plum); }
.tip-title { font-size: 15px; font-weight: 700; color: var(--fg); }
.tip-text { font-size: 13px; color: var(--fg); line-height: 19px; opacity: 0.85; }

.row-card { padding: 14px; border-radius: 22px; background-color: var(--surface); border: 1px solid var(--border); transition: background-color 160ms ease-out, scale 140ms ease-out; }
.row-card:hover { background-color: var(--surface-alt); }
.row-card:active { scale: 0.98; }

/* ── Кнопки ───────────────────────────────────────────────────────── */

.btn { padding: 11px 16px; border-radius: 16px; background-color: var(--surface); color: var(--fg); font-size: 14px; font-weight: 600; icon-color: var(--fg); icon-size: 19px; transition: background-color 160ms ease-out, scale 140ms ease-out, box-shadow 200ms ease-out; }
.btn:hover { background-color: var(--surface-alt); }
.btn:active { scale: 0.96; }
.btn.big { padding: 15px 20px; border-radius: 20px; font-size: 16px; }
.btn.primary { background: linear-gradient(135deg, var(--rose), var(--plum)); color: #ffffff; icon-color: #ffffff; box-shadow: 0 8px 22px var(--shadow-strong); }
.btn.primary:hover { background: linear-gradient(135deg, var(--rose-2), var(--plum)); }
.btn.primary:disabled { background: var(--surface-alt); color: var(--faint); icon-color: var(--faint); box-shadow: 0 0 0 #00000000; }
.btn.soft { background-color: var(--rose-soft); color: var(--rose); icon-color: var(--rose); }
.btn.soft:hover { background-color: var(--rose-line); color: #ffffff; icon-color: #ffffff; }
.btn.ghost { background-color: #00000000; border: 1px solid var(--border); color: var(--muted); icon-color: var(--muted); }
.btn.ghost:hover { background-color: var(--hover); }
.btn.flat { background-color: #00000000; icon-color: var(--muted); }
.btn.flat:hover { background-color: var(--hover); }
.btn.round { padding: 9px; border-radius: 999px; }
.btn.tb-btn { padding: 9px; icon-size: 22px; icon-color: var(--muted); }
.btn.nav-btn { padding: 8px; icon-size: 24px; icon-color: var(--fg); }
.btn.chip { padding: 7px 14px; border-radius: 999px; font-size: 13px; background-color: var(--rose-soft); color: var(--rose); }
.btn.danger { background-color: #e5484d1f; color: var(--danger); icon-color: var(--danger); }
.btn.danger-solid { background-color: var(--danger); color: #ffffff; }
.btn.step { padding: 8px; background-color: var(--surface-alt); icon-color: var(--fg); icon-size: 18px; }
.btn.step:disabled { icon-color: var(--faint); }

.btn.pick { padding: 8px 13px; border-radius: 999px; font-size: 13px; font-weight: 500; background-color: var(--surface-alt); color: var(--fg); icon-size: 17px; icon-color: var(--muted); border: 1px solid #00000000; }
.btn.pick.rose.on { background-color: var(--rose); color: #ffffff; icon-color: #ffffff; }
.btn.pick.plum.on { background-color: var(--plum); color: #ffffff; icon-color: #ffffff; }
.btn.pick.gold.on { background-color: var(--gold); color: #ffffff; icon-color: #ffffff; }

Toggle { width: 46px; height: 28px; background: var(--pressed); color: #ffffff; accent-color: var(--rose); border-radius: 14px; transition: background-color 200ms ease-out; }
Toggle:checked { background: var(--rose); }
Slider { height: 6px; background: var(--pressed); color: var(--plum); accent-color: var(--plum); border-radius: 3px; }

/* ── Календарь ────────────────────────────────────────────────────── */

.cal-card { padding: 14px; border-radius: 28px; background-color: var(--surface); border: 1px solid var(--border); box-shadow: 0 12px 30px var(--shadow); }
.month-title { font-size: 18px; font-weight: 700; color: var(--fg); }
.wd { height: 24px; align-items: center; justify-content: center; }
.wd-text { font-size: 12px; font-weight: 600; color: var(--muted); }
.wd-text.weekend { color: var(--rose); }

.cell-slot { justify-content: center; align-items: center; }
.cell { border-radius: 999px; justify-content: center; align-items: center; background-color: #00000000; border: 2px solid #00000000; transition: background-color 220ms ease-out, border-color 220ms ease-out, scale 200ms spring(420, 22); }
.cell:hover { background-color: var(--hover); }
.cell:active { scale: 0.9; }
.cell.period { background: linear-gradient(135deg, var(--rose), var(--rose-2)); box-shadow: 0 4px 12px var(--shadow); }
.cell.predicted { border: 2px dashed var(--rose-line); background-color: var(--rose-soft); }
.cell.fertile { background-color: var(--teal-soft); }
.cell.ovul { background-color: var(--teal); box-shadow: 0 4px 14px var(--teal-soft); }
.cell.off { opacity: 0.35; }
.cell.sel { scale: 1.1; border-color: var(--fg); }
.cell-num { font-size: 15px; font-weight: 500; color: var(--fg); }
.cell-num.strong { color: #ffffff; font-weight: 700; }
.cell-num.rose { color: var(--rose); font-weight: 700; }
.cell-num.teal { color: var(--teal); font-weight: 700; }
.cell-num.today { color: var(--plum); font-weight: 800; }
.cell-num.off { color: var(--faint); }
.dot { width: 5px; height: 5px; border-radius: 3px; }
.dot.none { background-color: #00000000; }
.dot.today { background-color: var(--plum); }
.dot.note { background-color: var(--gold); }

.legend { padding: 4px 6px; }
.legend-dot { width: 14px; height: 14px; border-radius: 7px; }
.legend-dot.period { background: linear-gradient(135deg, var(--rose), var(--rose-2)); }
.legend-dot.predicted { border: 2px dashed var(--rose-line); }
.legend-dot.fertile { background-color: var(--teal-soft); border: 1px solid var(--teal); }
.legend-dot.ovul { background-color: var(--teal); }
.legend-text { font-size: 12px; color: var(--muted); }

.event { padding: 11px 14px; border-radius: 18px; background-color: var(--surface); border: 1px solid var(--border); }
.event-when { font-size: 13px; font-weight: 600; color: var(--muted); }

/* ── Карточка дня ─────────────────────────────────────────────────── */

.scrim { background-color: var(--scrim); }
.sheet { border-radius: 30px 30px 0px 0px; background-color: var(--menu-bg); box-shadow: 0 -10px 40px var(--shadow-strong); }
.sheet.wide { border-radius: 30px; border: 1px solid var(--border); }
.sheet-inner { padding: 10px 60px 22px 60px; }
.grabber { width: 44px; height: 5px; border-radius: 3px; background-color: var(--border); }
.sheet-title { font-size: 24px; font-weight: 800; color: var(--fg); }
.status { padding: 8px 14px; border-radius: 14px; background-color: var(--surface-alt); }
.status.rose { background-color: var(--rose-soft); }
.status.rose-o { background-color: var(--rose-soft); border: 1px dashed var(--rose-line); }
.status.teal { background-color: var(--teal-soft); }
.status-text { font-size: 13px; font-weight: 600; color: var(--fg); }
.period-row { padding: 12px 14px; border-radius: 20px; background-color: var(--surface-alt); border: 1px solid #00000000; transition: background-color 220ms ease-out, border-color 220ms ease-out; }
.period-row.on { background-color: var(--rose-soft); border-color: var(--rose-line); }

/* ── Статистика ───────────────────────────────────────────────────── */

.stat { padding: 16px; border-radius: 22px; background-color: var(--surface); border: 1px solid var(--border); }
.stat-value { font-size: 30px; font-weight: 800; color: var(--fg); letter-spacing: -0.5px; }
.stat-unit { font-size: 14px; font-weight: 600; color: var(--muted); padding: 0px 0px 5px 0px; }
.stat.plum .stat-value { color: var(--plum); }
.stat.rose .stat-value { color: var(--rose); }
.stat.teal .stat-value { color: var(--teal); }
.stat.gold .stat-value { color: var(--gold); }
.cycle-row { padding: 14px; border-radius: 20px; background-color: var(--surface); border: 1px solid var(--border); }

.empty { padding: 36px 18px; border-radius: 28px; background-color: var(--surface); border: 1px solid var(--border); }
.hero-badge { width: 76px; height: 76px; border-radius: 38px; background-color: var(--rose-soft); justify-content: center; align-items: center; }
.hero-badge.danger { background-color: #e5484d24; }
.hero-badge.danger .hero-ic { icon-color: var(--danger); }
.hero-ic { icon-size: 36px; icon-color: var(--rose); }

/* ── Настройки ────────────────────────────────────────────────────── */

.group { border-radius: 24px; background-color: var(--surface); border: 1px solid var(--border); }
.group.pad { padding: 18px; }
.sep-wrap { padding: 0px 16px 0px 62px; }
.sep { height: 1px; background-color: var(--border); }
.set-row { padding: 14px 16px; }
.set-badge { width: 34px; height: 34px; border-radius: 11px; background-color: var(--plum-soft); justify-content: center; align-items: center; }
.set-ic { icon-size: 19px; icon-color: var(--plum); }
.step-value { width: 44px; align-items: center; }
.step-num { font-size: 18px; font-weight: 700; color: var(--fg); text-align: center; }
.step-unit { font-size: 10px; color: var(--muted); text-align: center; }
.scale-value { font-size: 18px; font-weight: 800; color: var(--plum); }
.segs { padding: 4px; border-radius: 18px; background-color: var(--surface-alt); }
.seg { padding: 10px 6px; border-radius: 14px; flex-grow: 1; align-items: center; background-color: #00000000; transition: background-color 240ms ease-out, box-shadow 240ms ease-out; }
.seg.on { background-color: var(--surface); box-shadow: 0 3px 10px var(--shadow); }
.seg-ic { icon-size: 21px; icon-color: var(--muted); }
.seg-ic.on { icon-color: var(--plum); }
.seg-text { font-size: 12px; color: var(--muted); }
.seg-text.on { color: var(--fg); font-weight: 700; }

.dialog { width: 340px; padding: 22px; border-radius: 28px; background-color: var(--menu-bg); border: 1px solid var(--border); box-shadow: 0 18px 50px var(--shadow-strong); }

/* ── Знакомство ───────────────────────────────────────────────────── */

.intro-top { height: 24px; }
.intro-hero { width: 128px; height: 128px; border-radius: 64px; background: linear-gradient(135deg, var(--rose), var(--plum)); justify-content: center; align-items: center; box-shadow: 0 18px 44px var(--shadow-strong); }
.intro-hero-ic { icon-size: 64px; icon-color: #ffffff; }
.intro-title { font-size: 34px; font-weight: 800; color: var(--fg); letter-spacing: -0.6px; }
.intro-text { font-size: 15px; color: var(--muted); }
.step-dot { width: 8px; height: 8px; border-radius: 4px; background-color: var(--border); transition: width 320ms spring(380, 26), background-color 220ms ease-out; }
.step-dot.on { width: 26px; background-color: var(--rose); }

/* ── Сообщения ────────────────────────────────────────────────────── */

.toast-wrap { width: 100%; height: 100%; padding: 0px 16px 96px 16px; }
.toast { padding: 12px 18px; border-radius: 16px; background-color: var(--fg); box-shadow: 0 10px 28px var(--shadow-strong); max-width: 420px; }
.toast-text { font-size: 14px; color: var(--bg); }
.fill { width: 100%; }
