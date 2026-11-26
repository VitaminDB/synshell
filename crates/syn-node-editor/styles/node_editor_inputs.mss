/* Input-виджеты внутри ноды.
 * Единая стилистика: компактные размеры, тёмные surface'ы, accent на focus.
 * Все интерактивные поля приведены к высоте --node-input-h (26px) для
 * визуальной согласованности. Checkbox/Toggle сохраняют родную геометрию,
 * центрируются Row.cross_axis_alignment(Center). */

/* ── TextField ──────────────────────────────────────────────────────── */
.node-input-text {
    background-color: #1B1D26;
    color: #E6E9EF;
    border: 1px solid rgba(255, 255, 255, 0.10);
    border-radius: 6px;
    width: 120px;
    height: 26px;
    font-size: 12px;
    padding-left: 8px;
    padding-right: 8px;
    transition: border-color 160ms ease-out, background-color 160ms ease-out;
}

.node-input-text:hover {
    border-color: rgba(255, 255, 255, 0.20);
}

.node-input-text:focus {
    border-color: var(--primary);
    background-color: #1F2230;
}

/* ── Slider ─────────────────────────────────────────────────────────── */
/* label-color / font-size — встроенный readout значения (Slider::show_value):
 * клик по числу открывает текстовый инлайн-ввод, caret-color — его курсор. */
.node-input-slider {
    width: 120px;
    height: 26px;
    accent-color: var(--primary);
    background-color: rgba(255, 255, 255, 0.10);
    label-color: var(--ne-text-muted, #98A0AD);
    caret-color: var(--primary);
    font-size: 11px;
    transition: accent-color 160ms ease-out;
}

.node-input-slider:hover {
    accent-color: var(--primary-hover);
}

/* ── Checkbox ───────────────────────────────────────────────────────── */
.node-input-checkbox {
    background-color: #1B1D26;
    color: #E6E9EF;
    accent-color: var(--primary);
    border: 1px solid rgba(255, 255, 255, 0.18);
    transition: accent-color 160ms ease-out, background-color 160ms ease-out, border-color 160ms ease-out;
}

.node-input-checkbox:hover {
    accent-color: var(--primary-hover);
    border-color: rgba(255, 255, 255, 0.28);
}

.node-input-checkbox:checked {
    background-color: var(--primary);
    border-color: var(--primary);
}

/* ── Toggle (switch) ────────────────────────────────────────────────── */
.node-input-toggle {
    accent-color: var(--primary);
    background-color: rgba(255, 255, 255, 0.14);
    transition: accent-color 160ms ease-out, background-color 160ms ease-out;
}

.node-input-toggle:hover {
    background-color: rgba(255, 255, 255, 0.22);
}

/* ── Dropdown ───────────────────────────────────────────────────────── */
.node-input-dropdown {
    background-color: #1B1D26;
    color: #E6E9EF;
    border: 1px solid rgba(255, 255, 255, 0.10);
    border-radius: 6px;
    width: 120px;
    height: 26px;
    font-size: 12px;
    padding-left: 8px;
    padding-right: 8px;
    transition: border-color 160ms ease-out, background-color 160ms ease-out;
}

.node-input-dropdown:hover {
    border-color: rgba(255, 255, 255, 0.20);
}

.node-input-dropdown:focus {
    border-color: var(--primary);
}

/* ── SpinBox ────────────────────────────────────────────────────────── */
.node-input-spinbox {
    background-color: #1B1D26;
    color: #E6E9EF;
    border: 1px solid rgba(255, 255, 255, 0.10);
    border-radius: 6px;
    height: 26px;
    font-size: 12px;
    transition: border-color 160ms ease-out;
}

.node-input-spinbox:hover {
    border-color: rgba(255, 255, 255, 0.20);
}

.node-input-spinbox:focus {
    border-color: var(--primary);
}

/* ── ColorPicker ────────────────────────────────────────────────────── */
.node-input-color {
    background-color: #1B1D26;
    color: #E6E9EF;
    accent-color: var(--primary);
    border: 1px solid rgba(255, 255, 255, 0.10);
    border-radius: 6px;
    height: 26px;
    font-size: 11px;
    transition: border-color 160ms ease-out, background-color 160ms ease-out;
}

.node-input-color:hover {
    background-color: #1F2230;
    border-color: rgba(255, 255, 255, 0.20);
}

/* Многострочные текстовые поля в переформатированных flow-нодах */
.node-input-text.flow-node-longtext {
    width: 100%;
    height: auto;
    min-height: 62px;
    padding-top: 6px;
    padding-bottom: 6px;
}

.node-input-dropdown.flow-node-dropdown {
    width: 176px;
}
