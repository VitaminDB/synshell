/* Рукописные страницы (notes/ink): панель инструментов в духе Onyx —
 * крупные кнопки, выбранный инструмент залит цветом текста; на E Ink
 * (eink.mss) — чёрное на белом без наведений. */
.ink-view {
    background-color: var(--bg-window);
}
.ink-canvas {
    background-color: var(--bg-window);
}
.ink-topbar {
    padding: 4px 8px;
    background-color: var(--bg-shell);
    border-bottom: 1px solid var(--border-soft);
}
.ink-sidebar {
    padding: 8px 4px;
    background-color: var(--bg-shell);
    border-right: 1px solid var(--border-soft);
}
.ink-toolbar {
    gap: 2px;
}
.ink-tool {
    width: 44px;
    height: 44px;
    icon-size: 24px;
    border-radius: 12px;
    color: var(--text);
}
.ink-tool:hover {
    background-color: var(--surface-hover);
}
.ink-tool.active {
    color: var(--on-primary);
    background-color: var(--text);
}
.ink-sep-v {
    width: 1px;
    height: 28px;
    margin: 0px 6px;
    background-color: var(--border);
}
.ink-sep-h {
    width: 28px;
    height: 1px;
    margin: 6px 0px;
    background-color: var(--border);
}
.ink-color-btn {
    width: 44px;
    height: 44px;
    padding: 11px;
    border-radius: 12px;
}
.ink-swatch.plain {
    border: 1px solid var(--border-strong);
}
.ink-swatch.selected {
    border: 3px solid var(--text);
}
.ink-ring {
    border: 2px solid var(--text);
}
.ink-pop {
    background-color: var(--bg-shell);
    border: 1px solid var(--border-strong);
    border-radius: 14px;
    padding: 10px;
}
.ink-pop-body {
    gap: 4px;
}
.ink-pop-caption {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-muted);
    margin: 6px 4px 2px 4px;
}
.ink-pop-item {
    height: 44px;
    icon-size: 22px;
    font-size: 15px;
    border-radius: 10px;
    color: var(--text);
    padding: 0px 10px;
}
.ink-pop-item.active {
    color: var(--on-primary);
    background-color: var(--text);
}
.ink-width-cell, .ink-color-cell {
    width: 44px;
    height: 44px;
    border-radius: 10px;
}
.ink-width-cell.selected, .ink-color-cell.selected {
    border: 2px solid var(--text);
}
.ink-layer-row {
    padding: 2px 4px;
    border-radius: 10px;
}
.ink-layer-row.active {
    border: 2px solid var(--text);
}
.ink-layer-name {
    font-size: 14px;
    color: var(--text);
}
.ink-pagebar {
    height: 52px;
    padding: 4px 12px;
    background-color: var(--bg-shell);
    border-top: 1px solid var(--border-soft);
}
.ink-pagebar-btn {
    padding: 8px 14px;
    border-radius: 12px;
}
.ink-pagebar-btn.active {
    border: 2px solid var(--text);
}
.ink-pagebar-icon {
    icon-size: 20px;
    color: var(--text);
}
.ink-pagebar-num {
    font-size: 16px;
    font-weight: 600;
    color: var(--text);
}
.ink-pages {
    padding: 16px;
}
.ink-page-cell {
    padding: 6px;
    border-radius: 10px;
}
.ink-page-cell.current {
    border: 3px solid var(--text);
}
.ink-page-thumb {
    height: 220px;
    border: 1px solid var(--border-strong);
    background-color: #FFFFFF;
}
.ink-page-add {
    height: 260px;
    border: 2px dashed var(--border-strong);
}
.ink-page-add-icon {
    icon-size: 40px;
    color: var(--text);
}
.ink-page-num {
    font-size: 14px;
    color: var(--text);
}
.ink-text-editor {
    font-size: 18px;
}
