/* syngamepad — встроенная тема. Переменные (--fg, --accent, …) — из [appearance] config.toml;
 * своё — в ~/.config/synshell/gamepad.mss. Прозрачность элементов — `opacity` раскладки. */

Text { color: #ffffff; font-size: 20px; }

.gp-fill { flex-grow: 1; }
.gp-label { font-size: 20px; text-box-edge: text; color: #ffffff; }
.gp-icon { font-size: 22px; color: #ffffff; }

.gp-btn {
    background-color: #20232acc;
    border-width: 2px;
    border-color: #ffffff66;
}
.gp-pad {
    background-color: #20232a88;
    border-width: 2px;
    border-color: #ffffff44;
}
.gp-stick {
    background-color: #20232a99;
    border-width: 2px;
    border-color: #ffffff55;
}
.gp-knob {
    background-color: #e8eaedcc;
    border-width: 2px;
    border-color: #ffffffaa;
}
.gp-dpad {
    background-color: #20232a99;
    border-width: 2px;
    border-color: #ffffff55;
}
.gp-dir { border-radius: 8px; }
.gp-arrow { font-size: 30px; color: #ffffff; }
.gp-pressed { background-color: var(--accent); border-color: #ffffff; }

.gp-handle {
    border-radius: 20px;
    background-color: #20232a88;
    border-width: 1px;
    border-color: #ffffff44;
    opacity: 0.6;
}
.gp-handle-icon { font-size: 22px; color: #ffffff; }

/* Правка раскладки. */
.gp-edit-bg { background-color: #000000a0; }
.gp-selected { border-color: var(--accent); border-width: 3px; }
.gp-panel {
    border-radius: 16px;
    background-color: var(--panel-bg);
    border-width: 1px;
    border-color: #ffffff30;
    box-shadow: 0 6px 20px #00000080;
}
.gp-tool { border-radius: 10px; background-color: #ffffff1a; padding: 0 6px; }
.gp-tool:active { background-color: var(--accent); }
.gp-tool-label { font-size: 14px; color: var(--fg); text-box-edge: text; }
.gp-tool-title { font-size: 14px; font-weight: bold; color: var(--fg); }
