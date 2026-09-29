/* synmobile-shell — встроенная тема. Переменные (--bg, --fg, --accent, …)
 * приходят из [appearance] config.toml, как и у syndesktop-shell. */

Text { color: var(--fg); font-size: 16px; }
.grow { flex-grow: 1; }
.muted { color: var(--muted); icon-color: var(--muted); }

.statusbar {
    flex-grow: 1;
    background-color: var(--panel-bg);
    padding: 6px 14px;
}
.statusbar-text { font-size: 15px; }

.navbar {
    flex-grow: 1;
    background-color: var(--panel-bg);
    padding: 6px 12px;
}
.nav-button {
    flex-grow: 1;
    padding: 10px 0px;
    border-radius: 12px;
    background-color: #00000000;
}
.nav-button:hover { background-color: var(--hover); }
.nav-button:active { background-color: var(--accent); }
.nav-icon { icon-size: 26px; icon-color: var(--fg); }

.home {
    flex-grow: 1;
    background-color: var(--bg);
    padding: 16px;
}
.home-title { font-size: 22px; padding: 8px 6px 14px 6px; }
.app-tile {
    padding: 10px 4px;
    border-radius: 14px;
    background-color: #00000000;
}
.app-tile:hover { background-color: var(--hover); }
.app-tile:active { background-color: var(--accent); }
.app-icon { icon-size: 48px; icon-color: var(--fg); }
.app-name { font-size: 13px; }
