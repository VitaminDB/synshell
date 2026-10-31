/* syn-epdtest — «Тест экрана E Ink». Всегда светлая тема с чистыми чёрным и белым:
   тест судит о панели, не о палитре оболочки. Без transition — на E Ink каждая
   промежуточная перерисовка видна волной. */

.grow { flex-grow: 1; }
Text { color: #000000; font-size: 14px; }
.hint { font-size: 12px; color: #505050; }
.label { font-size: 13px; color: #303030; padding: 0px 2px 0px 6px; }
.status { font-size: 12px; color: #303030; font-family: monospace; }
.error { font-size: 12px; color: #c00000; }
.stat { font-size: 15px; color: #000000; }
.big { font-size: 64px; font-weight: 600; color: #000000; }

/* ── Окно ─────────────────────────────────────────────────────────── */

.window { background-color: #ffffff; }
.window-frame { border-radius: 10px; border: 1px solid #000000; background-color: #ffffff; }
.window-frame.full, .window-frame:window-maximized, .window-frame:window-fullscreen { border-radius: 0px; border-width: 0px; }
.body { background-color: #ffffff; }

.titlebar { height: 44px; padding: 0px 4px 0px 12px; border-bottom: 1px solid #000000; }
.drag { height: 44px; flex-grow: 1; align-items: center; }
.brand-ic { icon-size: 22px; icon-color: #000000; }
.app-title { font-size: 15px; font-weight: 600; }
.window-controls { height: 44px; padding: 0px 4px; align-items: center; }
SystemWindowControls { color: #000000; background-color: #e0e0e0; }

.panel { padding: 8px 12px; border-bottom: 1px solid #000000; }
.sep { width: 1px; height: 26px; background-color: #a0a0a0; margin: 0px 4px; }
.field { font-size: 13px; color: #000000; background-color: #ffffff; border: 1px solid #000000; border-radius: 8px; padding: 6px 8px; }

.tabs { padding: 6px 12px 0px 12px; border-bottom: 1px solid #000000; }

/* ── Кнопки ───────────────────────────────────────────────────────── */

.btn { padding: 7px 12px; border-radius: 8px; background-color: #ffffff; color: #000000; font-size: 13px; icon-color: #000000; icon-size: 18px; border: 1px solid #000000; }
.btn:pressed { background-color: #d0d0d0; }
.btn.flat { border-width: 0px; padding: 8px; icon-size: 22px; }
.btn.chip.on, .btn.primary { background-color: #000000; color: #ffffff; icon-color: #ffffff; }
.btn.primary:pressed { background-color: #404040; }
.btn.tab { border-radius: 8px 8px 0px 0px; border-bottom-width: 0px; padding: 8px 16px; font-size: 14px; }
.btn.tab.on { background-color: #000000; color: #ffffff; font-weight: 600; }

/* ── Страницы ─────────────────────────────────────────────────────── */

.page { padding: 12px 16px 24px 16px; }
.section { padding: 8px 0px 0px 0px; }
.section-title { font-size: 17px; font-weight: 600; }
.pic { border: 0px; }
.canvas { background-color: #ffffff; }

.steps { height: 150px; border: 1px solid #000000; }
.steps.small { height: 60px; }
.step { flex-grow: 1; justify-content: center; align-items: end; padding: 0px 0px 8px 0px; }
.step-label { font-size: 18px; font-weight: 600; color: #000000; text-align: center; }
.step.dark .step-label { color: #ffffff; }
.band-labels { width: 200px; }
.band-label { height: 32px; justify-content: center; }

.patch { flex-grow: 1; height: 110px; padding: 6px; align-items: end; border: 1px solid #000000; }
.patch-label { font-size: 12px; color: #000000; }
.patch.dark .patch-label { color: #ffffff; }

.half { border: 1px solid #000000; }
.text-col { padding: 10px; background-color: #ffffff; }
.text-col .sample { color: #000000; }
.text-col.dark { background-color: #000000; }
.text-col.dark .sample { color: #ffffff; }

.ghost-side { width: 420px; }
.ghost-word { height: 200px; border: 2px solid #000000; justify-content: center; align-items: center; background-color: #ffffff; }
.ghost-word.dark { background-color: #000000; }
.ghost-text { font-size: 96px; font-weight: 700; color: #000000; }
.ghost-word.dark .ghost-text { color: #ffffff; }

.list-row { padding: 10px 0px; border-bottom: 1px solid #c0c0c0; }
.row-title { font-size: 16px; font-weight: 600; }
.row-text { font-size: 13px; color: #202020; }

/* ── Редактор профилей ────────────────────────────────────────────── */

.dialog { padding: 8px; border: 2px solid #000000; border-radius: 8px; }
.param { width: 270px; padding: 6px 8px; border: 1px solid #a0a0a0; border-radius: 8px; }
.param-title { font-size: 13px; font-weight: 600; }
.param-value { font-size: 13px; font-family: monospace; }
.param-unset { font-size: 12px; color: #606060; }
.btn.flat.small { padding: 2px; icon-size: 16px; }
