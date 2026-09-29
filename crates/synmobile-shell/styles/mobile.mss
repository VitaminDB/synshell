/* synmobile-shell — стили поверх общего shell.mss (synshell-ui). Переменные
 * (--bg, --fg, --accent, --panel-bg, --radius…) приходят из [appearance].
 * Свои правки — ~/.config/synshell/mobile.mss. */

.grow { flex-grow: 1; }

/* ─── Домашний экран ─────────────────────────────────────────────────── */

/* Лёгкое затемнение обоев, чтобы подписи читались на любой картинке. */
.home-scrim { background: linear-gradient(180deg, #00000030, #00000008 40%, #00000040); }

.home-pages {
    accent-color: var(--fg);
    border-color: #ffffff40;
}

.home-page { padding: 24px 14px 12px 14px; }

.home-clock { font-size: 64px; font-weight: 300; color: #ffffff; text-shadow: 0px 2px 8px #00000060; }
.home-date { font-size: 17px; color: #ffffffd0; text-shadow: 0px 1px 4px #00000060; }

.home-card {
    padding: 18px;
    border-radius: 22px;
    background-color: var(--surface);
    opacity: 0.94;
    box-shadow: 0px 8px 24px #00000040;
}
.home-stat-icon { icon-size: 20px; icon-color: var(--accent); }
.home-stat-label { font-size: 15px; color: var(--fg); }
.home-stat-value { font-size: 15px; font-weight: 600; color: var(--fg); }

.home-app-body { padding: 10px 2px; }
.home-app {
    border-radius: 18px;
    background-color: #00000000;
    transition: background-color 180ms ease-out, scale 220ms spring(420, 28);
}
.home-app:hover { background-color: #ffffff18; }
.home-app:active { background-color: #ffffff30; scale: 0.94; }
.home-app-launching { animation: home-launch 900ms ease-in-out infinite; }
@keyframes home-launch {
    0% { opacity: 1; }
    50% { opacity: 0.45; }
    100% { opacity: 1; }
}
.home-app-icon { icon-size: 56px; }
.home-app-name { font-size: 12px; color: #ffffff; text-align: center; text-shadow: 0px 1px 3px #000000a0; }

/* ─── Всплывающие окна на телефоне ───────────────────────────────────── */

/* Нижний лист: во всю ширину, скруглён сверху, с ручкой. */
.popup-sheet-place { padding: 0px 0px 8px 0px; }
.popup-sheet {
    border-radius: 26px;
    padding: 18px 16px 20px 16px;
    max-height: 86%;
}

.menu-caption { font-size: 12px; color: var(--muted); padding: 8px 12px 2px 12px; }
.menu-radio { border-radius: var(--radius-sm); }
.menu-radio-on { background-color: var(--accent-soft); }
