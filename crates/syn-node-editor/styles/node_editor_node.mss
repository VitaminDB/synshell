/* Node card: карточка одной ноды в редакторе нод.
 * Стиль — тёмная карточка с тонкой границей и приподнятой тенью.
 * При выделении (.selected) рамка становится primary. */

/* Карточка ноды. Рамка реализована через inset box-shadow, а не через
 * настоящий border: DecoratedBox.post_build_display_list рисует border
 * ПОСЛЕ children, и любая точка порта оказалась бы под рамкой и выглядела
 * полупрозрачной. Inset shadow рендерится в build_display_list ДО детей —
 * children (PortDot'ы) рисуются поверх. */
.node-card {
    /* Карточка ноды shrink-to-fit'ится по контенту: header + body row.
       Width / height не задаются константой — Positioned пробрасывает loose
       constraints, measure_container активирует two-pass shrink-to-fit
       через `Dimension::FitContent` (syngui/src/mss/value.rs). */
    width: fit-content;
    height: fit-content;
    /* Цвет фона выносим в `--node-bg` (cascade), чтобы inline-style
       `--node-bg: <blended_tint>` мог per-instance переопределять цвет.
       Hover-state НЕ задаёт background-color → переменная остаётся
       той же при наведении и фон не мигает на дефолтный.
       Парсер syngui MSS не поддерживает fallback `var(--x, default)` —
       поэтому базовое значение задаём отдельным declaration'ом. */
    --node-bg: var(--ne-node-bg, #2A2D38);
    background-color: var(--node-bg);
    border-radius: 8px;
    box-shadow:
        0 6px 16px rgba(0, 0, 0, 0.35),
        inset 0 0 0 1px rgba(255, 255, 255, 0.08);
    transition: box-shadow 180ms ease-out, transform 180ms ease-out;
}

.node-card:hover {
    box-shadow:
        0 10px 24px rgba(0, 0, 0, 0.45),
        inset 0 0 0 1px rgba(255, 255, 255, 0.18);
}

.node-card.selected {
    box-shadow:
        0 10px 24px rgba(0, 0, 0, 0.45),
        inset 0 0 0 2px var(--primary);
}

/* Drop-shadow выключен пользователем через ContextMenu «Тень: ☐»,
   или автоматически у disabled-ноды (см. ниже). Border (inset)
   сохраняем — без него карточка теряет визуальные границы.
   `:hover` override НУЖЕН: базовый `.node-card:hover` принудительно
   ставит drop-shadow при наведении — без явного override на
   `.no-shadow:hover` тень бы возвращалась при hover. */
.node-card.no-shadow {
    box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.08);
}
.node-card.no-shadow:hover {
    box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.18);
}
.node-card.selected.no-shadow {
    box-shadow: inset 0 0 0 2px var(--primary);
}
.node-card.selected.no-shadow:hover {
    box-shadow: inset 0 0 0 2px var(--primary);
}

/* Disabled-нода: приглушена + всегда без drop-shadow. Все интерактивные
   элементы (drag по header'у, ContextMenu) продолжают работать, чтобы
   пользователь мог снова включить ноду. */
.node-card.disabled {
    opacity: 0.45;
    transition: opacity 180ms ease-out;
}

.node-card-icon {
    icon-size: 18px;
    color: var(--primary);
}

.node-card-title {
    color: var(--ne-text-strong, #F0F1F4);
    font-size: 13px;
    font-weight: 600;
}

.node-card-close {
    color: var(--ne-text-muted, #98A0AD);
    border-radius: 4px;
    transition: background-color 140ms ease-out, color 140ms ease-out;
}

.node-card-close:hover {
    background-color: rgba(255, 255, 255, 0.10);
    color: var(--ne-text-strong, #FFFFFF);
}

.node-card-body {
    background-color: var(--ne-node-header, #232631);
    border-radius: 0 0 8px 8px;
    padding-top: 6px;
    padding-bottom: 6px;
}

.node-card-divider {
    height: 1px;
    background-color: rgba(255, 255, 255, 0.06);
    margin-top: 4px;
    margin-bottom: 4px;
}

/* Лейблы портов */
.node-card-port-label-in,
.node-card-port-label-out {
    color: var(--ne-text, #B7BDC9);
    font-size: 11px;
    font-weight: 500;
}

.node-card-port-label-out {
    text-align: right;
}

.node-card-port-spacer {
    flex-grow: 1;
    background-color: transparent;
}

/* Поля ноды */
.node-card-field-label {
    color: var(--ne-text-muted, #98A0AD);
    font-size: 11px;
    font-weight: 500;
}

.node-card-field-spacer {
    flex-grow: 1;
    background-color: transparent;
}

.node-card-field-error {
    color: #E55353;
    font-size: 11px;
}

/* Вычисленные значения, показываемые внутри карточки. Output — крупно
 * и по центру; Add — мелко и приглушённо, выровнено по правому краю
 * под output-портом. */
.node-output-value {
    color: var(--primary);
    font-size: 18px;
    font-weight: 600;
    text-align: center;
    padding-top: 4px;
    padding-bottom: 4px;
}

.node-port-value {
    color: var(--ne-text-muted, #98A0AD);
    font-size: 11px;
    font-weight: 500;
    text-align: right;
}

/* ── Кнопка справки `?` в шапке ноды ────────────────────────────────── */
.node-card-help {
    width: 22px;
    height: 22px;
    border-radius: 5px;
    background-color: transparent;
    transition: background-color 140ms ease-out;
}

.node-card-help:hover {
    background-color: rgba(255, 255, 255, 0.10);
}

.node-card-help-icon {
    icon-size: 15px;
    color: var(--ne-text-muted, #98A0AD);
}

/* ── Красивые rich-tooltip карточки ─────────────────────────────────── */
.ne-rich-tooltip {
    background-color: #1D202A;
    border-color: rgba(255, 255, 255, 0.14);
    border-width: 1px;
    border-radius: 10px;
    padding: 10px 12px;
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.55);
}

.ne-tip-body {
    width: 310px;
    background-color: transparent;
}

.ne-tip-body-compact {
    width: 252px;
    background-color: transparent;
}

.ne-tip-icon {
    icon-size: 16px;
    color: var(--primary);
}

.ne-tip-title {
    color: #F4F6FA;
    font-size: 12px;
    font-weight: 600;
}

.ne-tip-badge {
    background-color: rgba(255, 255, 255, 0.08);
    border-radius: 4px;
}

.ne-tip-badge-text {
    color: #C7CEDB;
    font-size: 10px;
    font-weight: 600;
}

.ne-tip-desc {
    color: #B8C0CE;
    font-size: 11px;
    font-weight: 400;
}

.ne-tip-meta {
    color: #8892A4;
    font-size: 10px;
    font-weight: 500;
}

.ne-tip-callout {
    background-color: rgba(238, 94, 72, 0.10);
    border: 1px solid rgba(238, 94, 72, 0.28);
    border-radius: 6px;
}

.ne-tip-callout-title {
    color: #FF9A8A;
    font-size: 10px;
    font-weight: 600;
}

.ne-tip-callout-text {
    color: #E2E6EE;
    font-size: 11px;
    font-weight: 400;
}

/* ── Переформатированные ноды synaptix-flow ─────────────────────────── */
.flow-node-host {
    width: 280px;
    background-color: transparent;
}

.flow-node-host-wide {
    width: 332px;
    background-color: transparent;
}

.flow-field-label-cell {
    width: 118px;
    min-width: 118px;
    background-color: transparent;
}

.flow-field-control-cell {
    flex-grow: 1;
    background-color: transparent;
}

.flow-node-longtext {
    width: 100%;
    min-height: 58px;
    padding-top: 6px;
    padding-bottom: 6px;
}

.flow-wired-pill {
    background-color: rgba(255, 255, 255, 0.05);
    border: 1px solid rgba(255, 255, 255, 0.09);
    border-radius: 6px;
}

.flow-wired-pill-text {
    color: #9FA8B8;
    font-size: 11px;
    font-weight: 500;
}

.flow-banner {
    background-color: rgba(255, 255, 255, 0.04);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 6px;
}

.flow-banner-text {
    color: #AEB6C5;
    font-size: 11px;
    font-weight: 400;
}

.flow-placeholder-chip {
    background-color: rgba(238, 94, 72, 0.14);
    border: 1px solid rgba(238, 94, 72, 0.32);
    border-radius: 5px;
    transition: background-color 120ms ease-out;
}

.flow-placeholder-chip:hover {
    background-color: rgba(238, 94, 72, 0.26);
}

.flow-chip-text {
    color: #FFD2CA;
    font-size: 10px;
    font-weight: 600;
}

.flow-adv-toggle {
    background-color: rgba(255, 255, 255, 0.04);
    border: 1px solid rgba(255, 255, 255, 0.07);
    border-radius: 6px;
    transition: background-color 140ms ease-out;
}

.flow-adv-toggle:hover {
    background-color: rgba(255, 255, 255, 0.08);
}

.flow-adv-chevron {
    icon-size: 16px;
    color: #98A0AD;
}

.flow-adv-title {
    color: #B7BDC9;
    font-size: 11px;
    font-weight: 600;
}

.flow-adv-badge {
    background-color: rgba(238, 94, 72, 0.20);
    border-radius: 4px;
}

.flow-adv-badge-text {
    color: #FFB4A8;
    font-size: 10px;
    font-weight: 600;
}

.flow-adv-panel {
    background-color: rgba(0, 0, 0, 0.18);
    border: 1px solid rgba(255, 255, 255, 0.06);
    border-radius: 6px;
}
