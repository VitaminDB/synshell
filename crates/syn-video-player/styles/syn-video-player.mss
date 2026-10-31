/* syn-video-player — «Видео». Палитра (--bg, --fg, --accent, …) — из [appearance]. */

.grow { flex-grow: 1; }
.root { background: var(--bg); }
Text { color: var(--fg); font-size: 14px; }

/* библиотека */
.header { padding: 12px 12px 8px 16px; }
.title { font-size: 22px; font-weight: 600; }
.ib { width: 44px; height: 44px; border-radius: 22px; padding: 10px; transition: background-color 120ms ease-out, scale 160ms spring(500, 28); }
.ib:hover { background-color: var(--hover); }
.ib:active { background-color: var(--pressed); scale: 0.92; }
.ib-icon { font-size: 24px; color: var(--fg); }

.grid-pad { padding: 4px 12px 24px 12px; }
.card { padding: 6px; border-radius: 14px; transition: background-color 120ms ease-out, scale 160ms spring(500, 28); }
.card:hover { background-color: var(--hover); }
.card:active { background-color: var(--pressed); scale: 0.97; }
.pic-box { border-radius: 10px; overflow: hidden; }
.pic-bg { background: var(--surface); }
.pic-icon { font-size: 40px; color: var(--muted); }
.dur-place { padding: 6px; }
.dur { padding: 2px 6px; border-radius: 6px; background: #000000b3; }
.dur-text { font-size: 12px; font-weight: 600; color: #ffffff; }
.card-title { font-size: 13px; padding: 0px 2px; }

.empty { flex-grow: 1; padding: 24px; }
.empty-icon { font-size: 64px; color: var(--muted); }
.empty-text { color: var(--muted); text-align: center; padding: 12px 0px 0px 0px; }
.error { margin: 0px 12px 8px 12px; padding: 10px 14px; border-radius: 12px; background: var(--danger); }
.error-text { color: #ffffff; }

/* плеер */
.player-bg { background: #000000; }
.player-center { flex-grow: 1; }
.player-error-text { color: #ffffff; text-align: center; padding: 0px 24px; }
.player-title { font-size: 16px; font-weight: 600; color: #ffffff; flex-shrink: 1; }

/* чип «как идёт воспроизведение» */
.info-chip { padding: 4px 4px 4px 12px; border-radius: 999px; background: #000000a6; border: 1px solid #ffffff26; }
.chip-what { font-size: 12px; font-weight: 600; color: #ffffffe6; }
.chip-mode { padding: 3px 10px 3px 8px; border-radius: 999px; }
.chip-mode-hw { background: #2e7d32; }
.chip-mode-sw { background: #ef6c00; }
.chip-mode-wait { background: #ffffff33; }
.chip-mode-icon { font-size: 14px; color: #ffffff; }
.chip-mode-text { font-size: 12px; font-weight: 700; color: #ffffff; }

/* настройки */
.set-scrim { background: #00000080; }
.set-panel { width: 380px; max-width: 92%; background: var(--bg); border-radius: 18px 0px 0px 18px; box-shadow: -6px 0px 24px #00000066; }
.set-head { padding: 14px 8px 8px 20px; }
.set-title { font-size: 20px; font-weight: 600; }
.set-close { color: var(--fg); }
.set-body { padding: 4px 14px 24px 14px; }
.set-card { padding: 14px; border-radius: 14px; background: var(--surface); }
.set-sec-icon { font-size: 20px; color: var(--accent); }
.set-sec-title { font-size: 15px; font-weight: 600; }
.set-row-title { font-size: 14px; }
.set-hint { font-size: 12px; color: var(--muted); }

/* Лента миниатюр каталога над шкалой перемотки (палитра тёмная — лежит на кадре). */
.strip { padding: 0px 0px 4px 0px; }
.strip-thumb {
    width: 96px;
    height: 54px;
    border-radius: 8px;
    border: 2px solid rgba(255, 255, 255, 0.0);
    overflow: hidden;
    opacity: 0.6;
    transition: opacity 100ms ease, border-color 100ms ease;
}
.strip-thumb-sm { width: 64px; height: 40px; border-radius: 7px; }
.strip-thumb:hover { opacity: 0.9; }
.strip-thumb-active { opacity: 1; border-color: var(--accent); }
.strip-thumb-active:hover { opacity: 1; }
.strip-bg { background: rgba(255, 255, 255, 0.12); }
.strip-icon { icon-size: 22px; font-size: 22px; color: rgba(255, 255, 255, 0.7); }

/* Монтаж (палитра тёмная — экран чёрный, как у плеера). */
.ed-top { padding: 8px 10px 6px 6px; background: #000000; }
.ed-preview { flex-grow: 1; flex-shrink: 1; min-height: 0px; overflow: hidden; }
.ed-panel { padding: 10px 14px 16px 14px; background: #121212; }
.ed-track { height: 46px; }
.ed-seg { flex-grow: 1; border-radius: 6px; border: 2px solid rgba(255, 255, 255, 0.0); overflow: hidden; transition: border-color 100ms ease; }
.ed-src0 { background: #3d6fd8; }
.ed-src1 { background: #2e9c6a; }
.ed-src2 { background: #b8792a; }
.ed-src3 { background: #9a4fc2; }
.ed-seg-off { background: rgba(255, 255, 255, 0.10); }
.ed-seg-on { border-color: #ffffff; }
.ed-head { width: 2px; background: #ffffff; }
.ed-info { font-size: 13px; color: #ffffffe6; }
.ed-hint { font-size: 12px; color: #ffffff99; flex-shrink: 1; }
.ed-tool { width: 68px; padding: 6px 2px; border-radius: 12px; transition: background-color 120ms ease-out, scale 160ms spring(500, 28); }
.ed-tool:hover { background-color: rgba(255, 255, 255, 0.08); }
.ed-tool:active { background-color: rgba(255, 255, 255, 0.16); scale: 0.94; }
.ed-tool-icon { font-size: 24px; color: #ffffff; }
.ed-tool-text { font-size: 11px; color: #ffffffcc; }
.ed-tool-off { opacity: 0.35; }
.ed-ib { width: 40px; height: 40px; border-radius: 20px; padding: 8px; }
.ed-ib:hover { background-color: rgba(255, 255, 255, 0.10); }
.ed-ib-icon { font-size: 24px; color: #ffffff; }
.ed-btn { padding: 8px 16px; border-radius: 999px; background: var(--surface); transition: scale 160ms spring(500, 28); }
.ed-btn:active { scale: 0.95; }
.ed-btn-primary { background: var(--accent); }
.ed-btn-icon { font-size: 18px; color: var(--fg); }
.ed-btn-text { font-size: 14px; font-weight: 600; color: var(--fg); }
.ed-btn-primary .ed-btn-icon, .ed-btn-primary .ed-btn-text { color: #ffffff; }
.ed-dialog { width: 420px; max-width: 92%; padding: 20px; border-radius: 18px; background: var(--bg); box-shadow: 0px 8px 32px #00000080; }
.ed-file { font-size: 13px; color: var(--fg); }
.ed-progress { height: 6px; }
.ed-add-hint { font-size: 12px; color: var(--muted); padding: 0px 20px 8px 20px; }
.ed-add-row { padding: 6px; border-radius: 12px; }
.ed-add-row:hover { background-color: var(--hover); }
.ed-add-thumb { width: 96px; height: 54px; border-radius: 8px; overflow: hidden; }
.ed-dur-place { padding: 3px; }

/* Выбор звуковой дорожки (панель как у настроек). */
.track-row { padding: 10px 12px; border-radius: 12px; background: var(--surface); border: 2px solid transparent; }
.track-row:hover { background: var(--hover); }
.track-row-active { border-color: var(--accent); }
.track-radio { font-size: 22px; icon-size: 22px; color: var(--muted); }
.track-row-active .track-radio { color: var(--accent); }

/* DLSS 5: зелёный NVIDIA — кадры идут через модель */
.chip-mode-dlss { background: #5e8f00; }
.info-chip-dlss { padding: 4px; }
.set-dlss-state { color: var(--accent); }
.set-dlss-open { align-self: flex-start; }

/* Субтитры: кнопка в шапке, панель (дорожки, синхронизация, оформление). */
.sub-btn-off { opacity: 0.5; }
.sub-scrim { background: #00000038; }
.sub-panel { width: 400px; }
.sub-panel .track-row { background: var(--bg); }
.sub-panel .track-row:hover { background: var(--hover); }
.sub-preview {
    width: 100%; height: 150px; padding: 10px; border-radius: 12px; overflow: hidden;
    background: linear-gradient(170deg, #8fb4d9 0%, #5d7f9e 45%, #2b3a2c 78%, #151a14 100%);
}
.sub-chip { padding: 6px 14px; border-radius: 999px; background: var(--bg); border: 2px solid transparent; }
.sub-chip-on { border-color: var(--accent); }
.sub-font { padding: 8px 12px; border-radius: 10px; background: var(--bg); border: 2px solid transparent; }
.sub-font:hover { background: var(--hover); }
.sub-font-on { border-color: var(--accent); }
.sub-font-name { font-size: 17px; font-weight: 600; }
.sub-swatch { width: 32px; height: 32px; padding: 3px; border-radius: 16px; border: 2px solid transparent; }
.sub-swatch-on { border-color: var(--accent); }
.sub-swatch-dot { flex-grow: 1; border-radius: 12px; border: 1px solid #8080808c; }
.sub-value { font-size: 12px; color: var(--muted); }
.sub-step { padding: 6px 10px; border-radius: 10px; }
.sub-delay-box { padding: 7px 4px; border-radius: 10px; background: var(--bg); }
.sub-delay { font-size: 14px; font-weight: 600; text-align: center; }
