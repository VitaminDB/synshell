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
