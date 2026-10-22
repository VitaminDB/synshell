/* syn-themeeditor — окно редактора тем (тема системы).
 * Предпросмотр рисуется в областях .te-scope-shell / .te-scope-settings
 * таблицами оболочки и «Параметров» с переменными черновика. */

.grow { flex-grow: 1; }

Text { color: var(--fg); font-size: var(--font-size); }

.te-frame {
    background-color: var(--content);
    border-radius: var(--window-radius);
    border-width: 1px;
    border-color: var(--border);
}
.te-frame.te-flat { border-radius: 0px; border-width: 0px; }
.te-frame:window-maximized, .te-frame:window-fullscreen { border-radius: 0px; border-width: 0px; }

.te-ic { icon-size: 20px; icon-color: var(--fg); }

/* ─── Заголовок ─── */
.te-titlebar {
    background-color: var(--titlebar);
    padding: 6px 8px 6px 12px;
    border-color: var(--divider);
    border-bottom-width: 1px;
    height: 48px;
}
.te-tb-left { width: 228px; height: 36px; align-items: center; }
.te-brand {
    padding: 6px;
    border-radius: 9px;
    background: linear-gradient(135deg, var(--accent), var(--accent-soft));
}
.te-brand-ic { icon-size: 18px; icon-color: var(--accent-fg); }
.te-brand-title { font-size: 15px; font-weight: 600; }
.te-drag-wrap { min-width: 40px; height: 36px; }
.te-drag { padding: 0px 8px; height: 36px; flex-grow: 1; justify-content: center; }
.te-title-row { height: 36px; }
.te-title { font-size: 14px; font-weight: 600; }
.te-dirty-dot { width: 8px; height: 8px; border-radius: 4px; background-color: var(--warning); }
.te-chip {
    padding: 2px 8px;
    border-radius: 10px;
    background-color: var(--surface-alt);
}
.te-chip-text { font-size: 11px; color: var(--muted); }
.te-window-controls { padding: 0px 0px 0px 6px; height: 36px; align-items: center; }
SystemWindowControls { color: var(--fg); background-color: var(--hover); }
.te-window { background-color: var(--content); }

/* ─── Кнопки ─── */
.te-btn {
    background-color: var(--surface-alt);
    color: var(--fg);
    icon-color: var(--fg);
    border-radius: var(--radius-sm);
    border-width: 1px;
    border-color: var(--divider);
    padding: 6px 12px;
    font-size: 13px;
    icon-size: 18px;
    transition: background-color 120ms ease-out;
}
.te-btn:hover { background-color: var(--hover); }
.te-btn:pressed { background-color: var(--pressed); }
.te-btn.te-primary { background-color: var(--accent); color: var(--accent-fg); icon-color: var(--accent-fg); border-color: var(--accent); }
.te-btn.te-primary:hover { background-color: var(--accent); opacity: 0.92; }
.te-btn.te-danger { background-color: var(--danger); color: #ffffff; icon-color: #ffffff; border-color: var(--danger); }
.te-btn.te-btn-on { background-color: var(--accent-soft); border-color: var(--accent); color: var(--fg); icon-color: var(--accent); }
.te-btn.te-flat { background-color: #00000000; border-color: #00000000; }
.te-btn.te-flat:hover { background-color: var(--hover); }
.te-btn.te-round { padding: 7px; border-radius: 18px; background-color: #00000000; border-color: #00000000; }
.te-btn.te-round:hover { background-color: var(--hover); }
.te-btn.te-off { opacity: 0.35; }
.te-btn.te-mini { padding: 4px; icon-size: 16px; }
.te-btn.te-wide { justify-content: center; }
.te-btn.te-danger-ic { icon-color: var(--danger); }
.te-btn.te-chip-btn { padding: 4px 10px; font-size: 12px; border-radius: 14px; }
.te-btn.te-chip-on { background-color: var(--accent-soft); border-color: var(--accent); }
.te-btn.te-chip-empty { color: var(--muted); }
.te-menu-item { justify-content: flex-start; }
.te-mini-space { width: 24px; }

.te-seg { font-size: 12px; accent-color: var(--accent); background-color: var(--surface-alt); color: var(--fg); border-color: var(--divider); }

/* ─── Колонки ─── */
.te-sidebar {
    width: 240px;
    background-color: var(--sidebar);
    border-color: var(--divider);
    border-right-width: 1px;
    padding: 12px 10px 8px 10px;
    gap: 10px;
}
.te-side-list { padding: 0px 0px 8px 0px; }
.te-side-group { font-size: 11px; font-weight: 600; color: var(--muted); padding: 12px 8px 4px 8px; }
.te-theme-item { padding: 8px; border-radius: var(--radius-sm); transition: background-color 100ms ease-out; }
.te-theme-item:hover { background-color: var(--hover); }
.te-theme-item-active { background-color: var(--accent-soft); }
.te-theme-item-active:hover { background-color: var(--accent-soft); }
.te-theme-name { font-size: 13px; }
.te-theme-item-active .te-theme-name { font-weight: 600; }
.te-theme-sub { font-size: 11px; color: var(--muted); }
.te-swatches { border-radius: 6px; }
.te-sw { width: 10px; height: 26px; }
.te-sw-first { border-top-left-radius: 6px; border-bottom-left-radius: 6px; }
.te-sw-last { border-top-right-radius: 6px; border-bottom-right-radius: 6px; }
.te-side-footer { border-color: var(--divider); border-top-width: 1px; padding: 8px 0px 0px 0px; }
.te-new-menu { padding: 0px 0px 4px 0px; }

.te-center { padding: 14px 16px 16px 16px; }
.te-preview { flex-grow: 1; }
.te-pv-toolbar { min-height: 36px; }

.te-inspector {
    width: 420px;
    background-color: var(--sidebar);
    border-color: var(--divider);
    border-left-width: 1px;
    padding: 12px 12px 0px 12px;
}
.te-insp-scroll { padding: 2px 4px 24px 0px; }
.te-modes { font-size: 12px; }

.te-hint { font-size: 12px; color: var(--muted); }
.te-group-title { font-size: 13px; font-weight: 600; padding: 4px 4px 0px 4px; }
.te-card {
    background-color: var(--card);
    border-radius: var(--radius);
    border-width: 1px;
    border-color: var(--divider);
    padding: 6px;
}
.te-row { padding: 6px 6px; border-radius: var(--radius-sm); }
.te-row:hover { background-color: var(--hover); }
.te-row-label { font-size: 13px; }
.te-row-label.te-row-hit { color: var(--accent); font-weight: 600; }
.te-row-sub { font-size: 11px; color: var(--muted); }
.te-slider-row { padding: 8px 8px; }
.te-slider { accent-color: var(--accent); }
.te-hex {
    font-size: 12px;
    font-family: "monospace";
    background-color: var(--input-bg);
    border-color: var(--divider);
    border-radius: var(--radius-sm);
    padding: 4px 8px;
    color: var(--fg);
    caret-color: var(--accent);
}
.te-hex.te-inherited { color: var(--muted); }
.te-search, .te-wide-field {
    background-color: var(--input-bg);
    border-color: var(--divider);
    border-radius: var(--radius-sm);
    padding: 7px 10px;
    color: var(--fg);
    caret-color: var(--accent);
}
.te-checker { border-radius: 8px; background-color: var(--checker-a); }

.te-grade { padding: 2px 6px; border-radius: 8px; min-width: 52px; justify-content: center; }
.te-grade-text { font-size: 11px; font-weight: 600; }
.te-grade-good { background-color: #22c55e2e; }
.te-grade-good .te-grade-text { color: var(--success); }
.te-grade-mid { background-color: #eab3082e; }
.te-grade-mid .te-grade-text { color: var(--warning); }
.te-grade-bad { background-color: var(--danger-soft); }
.te-grade-bad .te-grade-text { color: var(--danger); }

.te-presets { padding: 8px 6px; flex-wrap: wrap; }
.te-preset { width: 26px; height: 26px; border-radius: 13px; border-width: 2px; border-color: #00000000; }
.te-preset-on { border-color: var(--fg); }

.te-picked {
    background-color: var(--card);
    border-radius: var(--radius);
    border-width: 1px;
    border-color: var(--accent);
    padding: 8px;
}
.te-pick-ic { icon-color: var(--accent); }
.te-sel-row { flex-wrap: wrap; }
.te-wall-preview { height: 48px; border-radius: var(--radius-sm); border-width: 1px; border-color: var(--divider); }

.te-layer-tabs { flex-wrap: wrap; }
.te-code-wrap { border-radius: var(--radius-sm); border-width: 1px; border-color: var(--divider); min-height: 240px; }
.te-code {
    font-size: 13px;
    font-family: "monospace";
    background-color: var(--input-bg);
    color: var(--fg);
    editor-bg: var(--input-bg);
    editor-fg: var(--fg);
    editor-gutter-bg: var(--input-bg);
    editor-gutter-fg: var(--muted);
    editor-cursor: var(--accent);
    editor-selection: var(--accent-soft);
    editor-current-line: var(--hover);
    editor-bracket-match: var(--accent-soft);
    editor-popup-bg: var(--surface);
    editor-popup-border: var(--border);
    editor-error: var(--danger);
    editor-warning: var(--warning);
    token-keyword: var(--accent);
    token-property: var(--accent);
    token-attribute: var(--warning);
    token-tag: var(--accent);
    token-type: var(--success);
    token-string: var(--success);
    token-number: var(--warning);
    token-constant: var(--warning);
    token-comment: var(--muted);
    token-function: var(--fg);
    token-operator: var(--muted);
    token-punctuation: var(--muted);
    token-variable: var(--danger);
}

.te-error { background-color: var(--danger-soft); border-radius: var(--radius-sm); padding: 6px 10px; }
.te-error-ic { icon-color: var(--danger); }
.te-error-text { font-size: 12px; color: var(--danger); }

/* ─── Слои ─── */
.te-toast-wrap { padding: 0px 0px 24px 0px; }
.te-toast {
    background-color: var(--surface);
    border-radius: 14px;
    border-width: 1px;
    border-color: var(--border);
    padding: 10px 16px;
    max-width: 560px;
    box-shadow: 0 8px 24px var(--shadow);
}
.te-toast-text { font-size: 13px; }
.te-scrim { background-color: var(--scrim); flex-grow: 1; }
.te-dialog {
    width: 400px;
    background-color: var(--surface);
    border-radius: var(--radius);
    border-width: 1px;
    border-color: var(--border);
    padding: 18px;
    box-shadow: 0 16px 40px var(--shadow);
}
.te-dialog-title { font-size: 16px; font-weight: 600; }
.te-dialog-body { font-size: 13px; color: var(--muted); }

/* ─── Телефон ─── */
.te-tabs-wrap { padding: 8px 10px; background-color: var(--titlebar); }
.te-tabs { font-size: 13px; }
.te-phone .te-sidebar, .te-phone .te-inspector { width: auto; border-width: 0px; }
.te-phone-tools { padding: 8px 12px 0px 12px; }
.te-phone .te-center { padding: 8px; }

/* ─── Предпросмотр ─── */
.te-pv-frame {
    border-radius: 14px;
    border-width: 1px;
    border-color: var(--divider);
    flex-grow: 1;
}
.te-pv-wall { flex-grow: 1; }
.te-pv-desk { flex-grow: 1; padding: 10px; gap: 14px; }
.te-pv-body { padding: 4px 6px; }
.te-pv-launcher { width: 300px; }
.te-pv-right { width: 300px; }
.te-pv-notif { width: 300px; }
.te-pv-bottom { padding: 0px 0px 6px 0px; }
.te-pv-meter-fill { width: 120px; }
.te-pv-win-pos { padding: 60px 30px 10px 300px; }
.te-pv-win-back-pos { padding: 10px 300px 80px 120px; }
.te-pv-compact .page-body { padding: 18px 20px; }
.te-win { box-shadow: 0 14px 40px #00000066; width: auto; height: auto; }
.te-win-back { opacity: 0.96; }
.te-win-title { padding: 0px 12px; min-height: 34px; }
.te-win-title-text { font-size: 12px; font-weight: 600; }
.te-win-dot { width: 12px; height: 12px; border-radius: 6px; background-color: #9aa0ac66; }
.te-win-dot-close { background-color: #e5484d; }
.te-win-body { flex-grow: 1; }
.te-pv-set-side { width: 200px; }
.te-pv-settings-pos { padding: 36px 56px; flex-grow: 1; }
.te-win-full { flex-grow: 1; }

/* Пипетка: выбранный элемент обведён, во время выбора — подсказка курсором. */
.te-pick-armed { cursor: crosshair; }
.te-pick-hit { glow: 0 0 0 2px #ff3d9a; border-radius: 10px; }
