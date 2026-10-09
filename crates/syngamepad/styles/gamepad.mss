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
