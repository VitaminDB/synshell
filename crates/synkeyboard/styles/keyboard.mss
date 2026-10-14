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
.key-label { font-size: 19px; text-box-edge: text; }

.key-special { background-color: #ffffff10; }
.key-special .key-label { font-size: 15px; text-box-edge: ink; }
.key-fn { background-color: #ffffff10; }
.key-fn .key-label { font-size: 13px; text-box-edge: ink; }
.key-f .key-label { font-size: 12px; }

/* Дополнительные ряды (⌨): отдельный блок другого фона над основной клавиатурой. */
.keyboard-fn {
    background-color: #00000040;
    border-radius: 10px;
    border-width: 1px;
    border-color: #ffffff18;
}

/* Буква над нажатой клавишей (как в Gboard). */
.key-pop {
    border-radius: 12px;
    background-color: var(--surface);
    border-width: 1px;
    border-color: #ffffff22;
    box-shadow: 0 4px 14px #00000070;
}
.key-pop-label { font-size: 30px; text-box-edge: text; }

.key-icon { font-size: 20px; color: var(--fg); }

/* Плавающая: окно с ручкой сверху (полоска — тащить; справа «вниз» и «спрятать»). */
.keyboard-float {
    border-radius: 14px;
    border-width: 1px;
    border-color: #ffffff22;
    box-shadow: 0 8px 28px #00000080;
}
.keyboard-float .keyboard { border-radius: 0 0 14px 14px; }
.keyboard-grip { background-color: var(--panel-bg); border-radius: 14px 14px 0 0; }
.keyboard-grip-pill { width: 56px; height: 5px; border-radius: 3px; background-color: #ffffff55; }
.keyboard-grip-btn { width: 26px; height: 26px; border-radius: 13px; background-color: #ffffff1e; }
.keyboard-grip-btn:active { background-color: var(--accent); }
.keyboard-grip-icon { font-size: 16px; color: var(--fg); }

.key-active { background-color: var(--accent); }
.key-lock { border-width: 2px; border-color: var(--fg); }
