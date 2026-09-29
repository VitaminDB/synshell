/* synkeyboard — встроенная тема. Переменные (--bg, --fg, --accent, …)
 * приходят из [appearance] config.toml; своё — в ~/.config/synshell/keyboard.mss */

Text { color: var(--fg); font-size: 18px; }

.keyboard {
    flex-grow: 1;
    background-color: var(--panel-bg);
    padding: 6px 4px;
}

.key {
    flex-grow: 1;
    border-radius: 8px;
    background-color: #ffffff22;
}
.key:hover { background-color: #ffffff33; }
.key:active { background-color: var(--accent); }
.key-fill { flex-grow: 1; }
.key-spacer { flex-grow: 1; background-color: #00000000; }
.key-label { font-size: 19px; }

.key-special { background-color: #ffffff10; }
.key-special .key-label { font-size: 15px; }
.key-fn { background-color: #ffffff10; }
.key-fn .key-label { font-size: 13px; }
.key-f .key-label { font-size: 12px; }

.key-active { background-color: var(--accent); }
.key-lock { border-width: 2px; border-color: var(--fg); }
