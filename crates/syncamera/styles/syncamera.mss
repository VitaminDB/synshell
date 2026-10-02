/* syncamera — «Камера». Тёмная всегда (как у камер телефонов); акцент — из [appearance]. */

.root { background: #000000; }
Text { color: #ffffff; font-size: 14px; }
.grow { flex-grow: 1; }

/* верх и низ */
.topbar { padding: 0px 8px; }
.ib { width: 44px; height: 44px; border-radius: 22px; padding: 0px; transition: background-color 120ms ease-out, scale 160ms spring(500, 28); }
.ib:active { background-color: #ffffff33; scale: 0.92; }
.ib-icon { font-size: 24px; color: #ffffff; }
.ib-on .ib-icon { color: #ffd54f; }
.chip { padding: 6px 12px; border-radius: 16px; border-width: 1px; border-color: #ffffff66; transition: background-color 120ms ease-out; }
.chip:active { background-color: #ffffff33; }
.chip-on { background: #ffd54f; border-color: #ffd54f; }
.chip-text { font-size: 13px; font-weight: 600; color: #ffffff; }
.chip-on .chip-text { color: #000000; }

/* превью */
.preview { background: #000000; }
.live { background-color: #000000; }
.grid-line { background: #ffffff59; }
.focus-ring { border-width: 2px; border-color: #ffffffcc; border-radius: 36px; }
.focus-ok { border-color: #ffd54f; }
.focus-bad { border-color: #ff8a80; }
.lock-pill { padding: 3px 8px; border-radius: 10px; background: #ffd54f; }
.lock-icon { font-size: 14px; color: #000000; }
.lock-text { font-size: 11px; font-weight: 600; color: #000000; }
.blink { background: #000000; }
.center-ov { width: 100%; height: 100%; }
.countdown { font-size: 96px; font-weight: 300; color: #ffffff; }
.status { padding: 10px 16px; border-radius: 14px; background: #000000aa; max-width: 300px; }
.status-text { font-size: 14px; color: #ffffff; text-align: center; }
.zoom-big { padding: 8px 16px; border-radius: 18px; background: #000000aa; }
.ev-bar { padding: 4px 12px; border-radius: 22px; background: #00000088; }
.ev-icon { font-size: 20px; color: #ffd54f; }
.ev-text { font-size: 12px; width: 40px; color: #ffffff; }
.rec-pill { padding: 5px 12px; border-radius: 14px; background: #000000aa; }
.rec-dot { width: 10px; height: 10px; border-radius: 5px; background: #ff3b30; }
.rec-dot-paused { background: #ffffff88; }
.rec-text { font-size: 15px; font-weight: 600; color: #ffffff; }

/* низ: зум, режимы, затвор */
.bottombar { padding: 0px 0px 8px 0px; }
.zoom-row { padding: 4px; border-radius: 22px; background: #ffffff1a; }
.zoom-btn { width: 36px; height: 36px; border-radius: 18px; padding: 8px 0px; background: #00000066; transition: background-color 120ms ease-out, scale 160ms spring(500, 28); }
.zoom-btn:active { scale: 0.9; }
.zoom-on { width: 46px; background: #000000aa; }
.zoom-text { font-size: 12px; font-weight: 600; color: #ffffff; text-align: center; width: 100%; }
.zoom-on .zoom-text { color: #ffd54f; font-size: 13px; }
.mode { padding: 7px 12px; border-radius: 16px; transition: background-color 160ms ease-out; }
.mode-sel { background: #ffffff26; }
.mode-text { font-size: 14px; color: #ffffffb3; }
.mode-on { color: #ffd54f; font-weight: 600; }
.shutter-row { width: 100%; padding: 6px 12px; }
.side-btn { width: 56px; height: 56px; border-radius: 28px; padding: 0px; background: #ffffff26; }
.shutter { width: 78px; height: 78px; border-radius: 39px; border-width: 4px; border-color: #ffffff; padding: 5px; transition: scale 140ms spring(520, 26); }
.shutter:active { scale: 0.9; }
.shutter-busy { border-color: #ffffff66; }
.shutter-in { width: 60px; height: 60px; border-radius: 30px; background: #ffffff; transition: background-color 160ms ease-out; }
.shutter-in:active { background-color: #dddddd; }
.shutter-rec { background: #ff3b30; }
.shutter-stop { width: 30px; height: 30px; border-radius: 6px; background: #ff3b30; }
.shutter-night { background: #ffd54f; }
.night-icon { font-size: 26px; color: #000000; }
.thumb { width: 56px; height: 56px; border-radius: 28px; border-width: 2px; border-color: #ffffff; background: #222222; }
.thumb-img { width: 56px; height: 56px; }
.thumb-play { font-size: 22px; color: #ffffff; }
.thumb-empty { font-size: 22px; color: #ffffff66; }

/* профи */
.pro-row { padding: 4px; border-radius: 16px; background: #00000088; width: 100%; }
.pro-chip { padding: 6px 8px; border-radius: 12px; min-width: 58px; }
.pro-on { background: #ffffff26; }
.pro-name { font-size: 11px; color: #ffffffaa; }
.pro-val { font-size: 13px; font-weight: 600; color: #ffffff; }
.pro-on .pro-val { color: #ffd54f; }
.pro-slider { padding: 6px 12px; border-radius: 22px; background: #00000088; }
.pro-note { font-size: 13px; color: #ffffffaa; padding: 8px 12px; }
.wb-icon { font-size: 22px; color: #ffffff; }

/* настройки */
.scrim { background: #00000099; width: 100%; height: 100%; }
.sheet { background: #1c1c1e; border-radius: 24px 24px 0px 0px; padding: 12px 18px 0px 18px; }
.sheet-title { font-size: 20px; font-weight: 600; }
.set-head { font-size: 13px; font-weight: 600; color: #ffd54f; padding: 14px 0px 4px 0px; }
.set-row { padding: 8px 0px; }
.set-label { font-size: 15px; }
.set-sub { font-size: 12px; color: #ffffff99; }

/* просмотр */
.viewer-bg { background: #000000; width: 100%; height: 100%; }
.viewer-top { padding: 8px 4px; background: #000000aa; }
.viewer-title { font-size: 15px; font-weight: 600; }
.viewer-sub { font-size: 12px; color: #ffffffaa; }
.viewer-arrows { padding: 0px 4px; width: 100%; }
.viewer-note { color: #ffffffaa; }
.viewer-controls { padding: 8px 12px 18px 12px; background: #000000aa; }
.viewer-time { font-size: 12px; color: #ffffffcc; }
.info-card { margin: 4px 12px; padding: 12px 14px; border-radius: 14px; background: #1c1c1eee; }
.info-name { font-size: 14px; font-weight: 600; }
.info-sub { font-size: 12px; color: #ffffffaa; }
.dlg { width: 300px; padding: 20px; border-radius: 22px; background: #2c2c2e; }
.dlg-title { font-size: 18px; font-weight: 600; }
.dlg-text { font-size: 14px; color: #ffffffbb; }
.dlg-btn { padding: 9px 16px; border-radius: 18px; }
.dlg-btn:active { background-color: #ffffff22; }
.dlg-danger { background: #ff3b30; }
.dlg-btn-text { font-size: 14px; font-weight: 600; }

.toast-place { padding: 0px 16px 260px 16px; width: 100%; height: 100%; }
.toast { padding: 10px 16px; border-radius: 999px; background: #ffffffee; }
.toast-text { font-size: 13px; color: #000000; }
.switch-dim { background: #000000b3; }
.shutter-col { padding: 12px 6px; }
.vbar .chip { padding: 5px 7px; }
.vbar .chip-text { font-size: 11px; }
.hint { padding: 6px 8px 6px 12px; border-radius: 20px; background: #000000b3; max-width: 360px; }
.hint-night { padding: 8px 14px; }
.hint-icon { font-size: 20px; color: #ffd54f; }
.hint-text { font-size: 13px; color: #ffffff; max-width: 170px; }
.hint-btn { width: 36px; height: 36px; padding: 0px; border-radius: 18px; }
.hint-btn .ib-icon { font-size: 20px; }

/* выбор задней камеры */
.lens-pill { padding: 0px 10px 0px 8px; border-radius: 18px; background: #000000a6; border-width: 1px; border-color: #ffffff2e; transition: background-color 140ms ease-out, scale 160ms spring(500, 28); }
.lens-pill:active { scale: 0.95; }
.lens-pill-open { background: #2c2c2ef2; border-color: #ffd54f99; }
.lens-icon { font-size: 18px; color: #ffd54f; }
.lens-title { font-size: 13px; font-weight: 600; color: #ffffff; }
.lens-chevron { font-size: 20px; color: #ffffffb3; }
.lens-menu { padding: 12px 8px 8px 8px; border-radius: 22px; background: #1f1f21f5; border-width: 1px; border-color: #ffffff1f; box-shadow: 0 10px 30px #00000099; }
.lens-head { font-size: 12px; font-weight: 600; color: #ffffff80; padding: 0px 10px 6px 10px; }
.lens-row { padding: 8px 10px; border-radius: 16px; transition: background-color 120ms ease-out; }
.lens-row:active { background-color: #ffffff14; }
.lens-row-sel { background: #ffd54f1f; }
.lens-badge { width: 40px; height: 40px; border-radius: 20px; background: #ffffff1a; }
.lens-badge-off { background: #ffffff0a; }
.lens-row-icon { font-size: 22px; color: #ffffff; }
.lens-row-sel .lens-row-icon { color: #ffd54f; }
.lens-badge-off .lens-row-icon { color: #ffffff4d; }
.lens-row-title { font-size: 15px; font-weight: 600; color: #ffffff; }
.lens-off { color: #ffffff59; }
.lens-row-sub { font-size: 12px; color: #ffffff99; }
.lens-fault { color: #ff8a80; }
.lens-check { font-size: 22px; color: #ffd54f; }
.lens-block { font-size: 20px; color: #ffffff4d; }
