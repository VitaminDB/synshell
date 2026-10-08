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

/* Рабочие столы: точки вверху, активный — вытянутый. */
.home-ws { padding: 6px 10px; border-radius: 999px; background-color: #00000030; }
.home-ws-hit { padding: 4px 2px; }
.home-ws-dot {
    width: 7px; height: 7px; border-radius: 4px;
    background-color: #ffffff50;
    transition: width 220ms spring(420, 28), background-color 160ms ease-out;
}
.home-ws-dot-busy { background-color: #ffffffa0; }
.home-ws-dot-on { width: 20px; background-color: #ffffff; }

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

/* ─── Экран ресурсов (страница 0) ────────────────────────────────────── */

.res-page { padding: 18px 12px 24px 12px; }
.res-clock { font-size: 40px; font-weight: 300; color: #ffffff; text-shadow: 0px 2px 8px #00000060; padding: 0px 10px 0px 4px; }
.res-date { font-size: 14px; color: #ffffffc8; padding: 0px 0px 8px 0px; }
.res-card {
    padding: 14px;
    border-radius: 22px;
    background-color: var(--surface);
    opacity: 0.95;
    box-shadow: 0px 8px 24px #00000040;
}
.res-card-icon { icon-size: 18px; icon-color: var(--accent); }
.res-card-title { font-size: 14px; font-weight: 600; color: var(--fg); }
.res-ring-value { font-size: 19px; font-weight: 600; color: var(--fg); }
.res-ring-sub { font-size: 11px; color: var(--muted); }
.res-ring-unit { font-size: 10px; color: var(--muted); }
.res-k { font-size: 12px; color: var(--muted); }
.res-net-lead { width: 20px; }
.res-net-icon { font-size: 18px; icon-color: var(--fg); }
.res-v { font-size: 12px; color: var(--fg); }
.res-big { font-size: 28px; font-weight: 600; color: var(--fg); }
.res-core-bg { width: 16px; border-radius: 5px; background-color: var(--surface-alt); }
/* без перехода высоты: данные раз в секунду, а переход 400 мс перерисовывал страницу почти непрерывно (SM-T295) */
.res-core-fill { width: 16px; border-radius: 5px; }
.res-tier-0 { background-color: #4fc3f7; }
.res-tier-1 { background-color: var(--accent); }
.res-tier-2 { background-color: #ffb74d; }
.res-tier-3 { background-color: #ef5350; }
.res-core-mhz { font-size: 9px; color: var(--muted); }
.res-section { font-size: 14px; font-weight: 600; color: #ffffff; text-shadow: 0px 1px 4px #00000080; padding: 6px 4px 0px 4px; }
.res-empty { font-size: 13px; color: #ffffffb0; padding: 4px; }
.res-rail-row { padding: 2px 2px 8px 2px; }
.res-app {
    width: 140px;
    padding: 18px 8px 10px 8px;
    border-radius: 20px;
    background-color: var(--surface);
    transition: scale 220ms spring(420, 26);
}
.res-app:active { scale: 0.95; }
.res-app-close-row { width: 140px; padding: 4px; }
.res-app-close {
    padding: 3px;
    border-radius: 999px;
    background-color: #00000040;
    transition: background-color 120ms ease-out, scale 200ms spring(420, 26);
}
.res-app-close:active { background-color: #ef5350; scale: 0.9; }
.res-app-close-icon { icon-size: 14px; icon-color: var(--fg); }
.res-app-icon { width: 44px; height: 44px; }
.res-app-name { font-size: 12px; font-weight: 600; color: var(--fg); text-align: center; }
.res-app-title { font-size: 10px; color: var(--muted); text-align: center; }
.res-app-badges { padding-top: 4px; }
.res-badge { padding: 2px 6px; border-radius: 999px; }
.res-badge-text { font-size: 10px; font-weight: 700; color: #ffffff; }
.res-badge-mem { background-color: #5c6bc0; }
.res-badge-cpu { background-color: #26a69a; }
.res-badge-hot { background-color: #ef5350; }

/* Половины строки — поровну, независимо от содержимого. */
.res-half { flex-grow: 1; flex-basis: 0px; }
