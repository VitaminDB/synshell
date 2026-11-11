/* synphone — «Телефон». Переменные палитры (--bg, --fg, --accent…) — из [appearance]. */

.root { background: var(--bg); flex-grow: 1; }
Text { color: var(--fg); font-size: 14px; }
.grow { flex-grow: 1; }
.muted { color: var(--muted); font-size: 13px; }
.empty { padding: 96px 16px 16px 16px; width: 100%; }
.empty-icon { font-size: 48px; color: var(--muted); }
.title { font-size: 24px; font-weight: 700; }
.tab-head { padding: 14px 18px 8px 18px; }
.section { font-size: 12px; font-weight: 600; color: var(--accent); padding: 14px 12px 4px 12px; }

.net { padding: 4px 4px 0px 16px; }
.net-ok { font-size: 12px; color: var(--muted); }
.net-bad { font-size: 12px; color: var(--danger); }

/* Аватары */
.av { border-radius: 999px; background: var(--surface-alt); }
.av-img { width: 100%; height: 100%; }
.av-s { width: 34px; height: 34px; }
.av-m { width: 42px; height: 42px; }
.av-l { width: 60px; height: 60px; }
.av-xl { width: 112px; height: 112px; }
.av-call { width: 128px; height: 128px; box-shadow: 0 10px 40px #00000066; }
.av-letter { font-size: 17px; font-weight: 600; color: #ffffff; }
.av-l .av-letter { font-size: 24px; }
.av-xl .av-letter, .av-call .av-letter { font-size: 48px; }
.av-icon { font-size: 22px; color: var(--muted); }
.av-xl .av-icon, .av-call .av-icon { font-size: 56px; }
.av-c0 { background: #5b7cfa; }
.av-c1 { background: #e0638a; }
.av-c2 { background: #2eb67d; }
.av-c3 { background: #f2994a; }
.av-c4 { background: #9b6dde; }
.av-c5 { background: #23a8c4; }
.av-c0 .av-icon, .av-c1 .av-icon, .av-c2 .av-icon, .av-c3 .av-icon, .av-c4 .av-icon, .av-c5 .av-icon { color: #ffffff; }
.av-add { background: var(--accent-soft); }
.av-add .av-icon { color: var(--accent); }
.av-rec { background: var(--accent-soft); }
.av-rec .av-icon { color: var(--accent); }

/* Кнопки-значки */
.ib { width: 40px; height: 40px; border-radius: 20px; padding: 8px; background-color: #00000000; transition: background-color 120ms ease-out; }
.ib:hover { background-color: var(--hover); }
.ib:active { background-color: var(--pressed); }
.ib-icon { font-size: 24px; color: var(--fg); }
.ib-accent .ib-icon { color: var(--accent); }
.row-btn .ib-icon { font-size: 22px; color: var(--accent); }

/* Набор */
.dial { padding: 0px 16px 8px 16px; }
.hints { width: 100%; max-width: 380px; padding: 0px 4px 6px 4px; }
.hints-empty { height: 4px; }
.hint-row { padding: 6px 10px; border-radius: var(--radius); background-color: #00000000; transition: background-color 120ms ease-out; }
.hint-row:hover { background-color: var(--hover); }
.hint-row:active { background-color: var(--pressed); }
.hint-name { font-size: 14px; font-weight: 600; }
.hint-num { font-size: 12px; color: var(--muted); }
.hint-add { font-size: 14px; color: var(--accent); font-weight: 600; }
.display-row { padding: 0px 8px 16px 8px; }
.display { font-size: 34px; max-width: 300px; }
.display-side { width: 44px; height: 44px; padding: 10px; }
.bs-icon { font-size: 24px; color: var(--muted); }
.key { width: 76px; height: 76px; border-radius: 38px; background: var(--surface-alt); transition: background-color 100ms ease-out, scale 160ms spring(500, 28); }
.key:hover { background-color: var(--hover); }
.key:active { background-color: var(--pressed); scale: 0.94; }
.key-digit { font-size: 28px; }
.key-letters { font-size: 9px; color: var(--muted); }
.call-row { padding: 18px 0px 10px 0px; }

/* Круглые кнопки */
.round { width: 64px; height: 64px; border-radius: 32px; padding: 18px; transition: scale 160ms spring(500, 28), background-color 140ms ease-out; }
.round:active { scale: 0.92; }
.round-big { width: 74px; height: 74px; border-radius: 37px; padding: 22px; }
.round-sm { width: 52px; height: 52px; border-radius: 26px; padding: 14px; }
.round-icon { font-size: 28px; color: #ffffff; }
.round-green { background: #2e9d48; }
.round-red { background: #e5383b; }
.round-ring { box-shadow: 0 0 0 10px #2e9d4833; }
.round-soft { background: var(--surface-alt); }
.round-soft .round-icon { color: var(--fg); }
.round-soft-on { background: var(--accent); }
.round-soft-on .round-icon { color: #ffffff; }
.round-label { font-size: 12px; color: var(--muted); }

/* Списки */
.list { padding: 0px 8px 16px 8px; }
.list-fab { padding: 0px 8px 96px 8px; }
.log-row { padding: 8px 10px; border-radius: var(--radius); background-color: #00000000; transition: background-color 120ms ease-out; }
.log-row:hover { background-color: var(--hover); }
.log-row:active { background-color: var(--pressed); }
.log-icon { font-size: 15px; color: var(--muted); }
.log-missed { color: var(--danger); }
.log-rec { font-size: 15px; color: var(--accent); }
.log-name { font-size: 15px; font-weight: 500; }
.log-name-missed { color: var(--danger); }
.log-sub { font-size: 12px; color: var(--muted); }
.chips { padding: 0px; }
.chip { padding: 5px 12px; border-radius: 999px; border: 1px solid var(--border); }
.chip-on { background: var(--accent-soft); border-color: var(--accent-soft); }
.chip-text { font-size: 12px; }

/* Контакты */
.search { padding: 0px 14px 6px 14px; }
.search-icon { font-size: 20px; color: var(--muted); }
.search-input { height: 40px; }
.favs { padding: 4px 12px 8px 12px; }
.fav { width: 72px; }
.fav-name { font-size: 12px; max-width: 70px; }
.fab-place { padding: 0px 20px 20px 0px; }
.fab { width: 58px; height: 58px; border-radius: 20px; background: var(--accent); padding: 16px; box-shadow: 0 6px 20px var(--shadow); transition: scale 160ms spring(500, 28); }
.fab:active { scale: 0.92; }
.fab-icon { font-size: 26px; color: #ffffff; }
.card-name { font-size: 26px; font-weight: 700; }

/* Страницы */
.page { background: var(--bg); flex-grow: 1; }
.page-header { padding: 6px 8px; }
.page-title { font-size: 19px; font-weight: 600; }
.header-act { padding: 6px 8px; }
.page-body { padding: 4px 14px 32px 14px; }
.card { border-radius: 16px; background: var(--surface); padding: 2px 0px; }
.set-row { padding: 12px 14px; }
.row-title { font-size: 15px; }
.row-hint { font-size: 12px; color: var(--muted); }
.row-badge { width: 36px; height: 36px; border-radius: 18px; background: var(--accent-soft); padding: 8px; }
.row-badge-icon { font-size: 20px; color: var(--accent); }
.row-chevron { font-size: 22px; color: var(--muted); }
.seg { padding: 3px; border-radius: 12px; background: var(--surface-alt); }
.seg-item { flex-grow: 1; padding: 7px 8px; border-radius: 9px; align-items: center; }
.seg-on { background: var(--accent); }
.seg-text { font-size: 13px; }
.seg-on .seg-text { color: #ffffff; font-weight: 600; }
.dir-line { padding: 0px 18px 6px 18px; }

/* Редактор */
.field { padding: 4px 6px 4px 12px; border-radius: 14px; background: var(--surface); }
.field-icon { font-size: 22px; color: var(--muted); }
.input { height: 44px; }
.kind { padding: 6px 10px; border-radius: 999px; background: var(--surface-alt); }
.kind-text { font-size: 12px; color: var(--accent); }
.add-row { padding: 10px 12px; }
.add-text { color: var(--accent); font-weight: 600; }
.pill { padding: 8px 14px; border-radius: 999px; background: var(--surface-alt); }
.pill-icon { font-size: 18px; color: var(--accent); }
.pill-text { font-size: 13px; }

/* Экран звонка — всегда тёмный (фото или градиент) */
.call-screen { flex-grow: 1; background: #0e1116; }
.call-bg { flex-grow: 1; background: linear-gradient(180deg, #1c2a3a 0%, #0f1720 55%, #090c10 100%); }
.photo-shade { flex-grow: 1; background: linear-gradient(180deg, #00000066 0%, #00000022 30%, #000000aa 62%, #000000ee 100%); }
.in-call { padding: 56px 22px 36px 22px; }
.call-info { width: 100%; }
.call-screen Text { color: #ffffff; }
.call-number { font-size: 30px; font-weight: 700; text-shadow: 0 2px 8px #00000099; }
.call-sub { font-size: 14px; color: #ffffffbb; }
.call-state { font-size: 16px; color: #ffffffcc; }
.rec-pill { padding: 3px 10px; border-radius: 999px; background: #e5383b33; }
.rec-dot { width: 8px; height: 8px; border-radius: 4px; background: #ff4d4f; }
.rec-text { font-size: 13px; color: #ffb3b3; }
.other-call { padding: 5px 12px; border-radius: 999px; background: #ffffff1f; }
.other-text { font-size: 13px; }
.call-buttons { width: 100%; padding: 4px 0px; }
.call-row-btns { width: 100%; }
.call-screen .round-soft { background: #ffffff24; }
.call-screen .round-soft .round-icon { color: #ffffff; }
.call-screen .round-soft-on { background: #ffffff; }
.call-screen .round-soft-on .round-icon { color: #0e1116; }
.call-screen .round-rec-on { background: #e5383b; }
.call-screen .round-label { color: #ffffffcc; }
.call-screen .key { background: #ffffff1f; }
.call-screen .key-letters { color: #ffffff99; }
.pill-glass { background: #ffffff24; }
.call-screen .pill-icon { color: #ffffff; }
.vis-box { padding: 0px 6px; }
.vis { height: 110px; color: #ffffff; accent-color: #4ade80; }
.vis-space { height: 8px; }
.vis-label { font-size: 11px; color: #ffffff88; }
.vis-them { color: #4ade80; }

/* Плашка «вернуться к звонку» */
.return-bar { padding: 10px 16px; background: #2e9d48; }
.return-icon { font-size: 20px; color: #ffffff; }
.return-text { font-size: 14px; color: #ffffff; font-weight: 600; }

/* Нижние вкладки */
.navbar { padding: 6px 0px 10px 0px; border-top-width: 1px; border-color: var(--border); background: var(--surface); }
.nb-item { padding: 4px 14px; }
.nb-pill { padding: 4px 20px; border-radius: 16px; }
.nb-pill-on { background: var(--accent-soft); }
.nb-icon { font-size: 22px; color: var(--fg); }
.nb-label { font-size: 11px; color: var(--muted); }
.nb-label-on { color: var(--fg); font-weight: 600; }
.nb-badge { min-width: 18px; height: 18px; border-radius: 9px; background: var(--danger); padding: 0px 5px; align-items: center; justify-content: center; }
.nb-badge-text { font-size: 10px; color: #ffffff; font-weight: 700; }

.toast-place { padding: 0px 16px 96px 16px; }
.toast { padding: 10px 16px; border-radius: 999px; background: var(--fg); box-shadow: 0 6px 20px var(--shadow); }
.toast-text { font-size: 13px; color: var(--bg); }

/* Своя рамка окна (на телефоне — плоская) */
.window-frame { border-radius: var(--window-radius); border: 1px solid var(--border); background: var(--bg); }
.window-frame.flat { border-radius: 0px; border-width: 0px; }
.window-frame:window-maximized, .window-frame:window-fullscreen { border-radius: 0px; border-width: 0px; }
.titlebar { height: 44px; background: var(--bg); }
.tb-left { height: 44px; padding: 0px 8px 0px 12px; align-items: center; }
.tb-btn { height: 44px; align-items: center; }
.brand { width: 26px; height: 26px; border-radius: 13px; background: #2e9d48; padding: 5px; }
.brand-icon { font-size: 16px; color: #ffffff; }
.brand-title { font-size: 13px; font-weight: 600; }
.tb-drag { height: 44px; }
.tb-drag-space { flex-grow: 1; height: 44px; min-width: 40px; padding: 0px 8px; align-items: center; justify-content: center; }
.window-controls { height: 44px; padding: 0px 6px 0px 4px; align-items: center; }
SystemWindowControls { color: var(--fg); background-color: var(--hover); }
