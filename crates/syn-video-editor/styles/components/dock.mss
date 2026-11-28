/* ── док-панели: группы — карточки на фоне шелла, между ними воздух ── */
.dock-area-pad {
    /* боковой отступ — внутри дока (DockArea::inset в ui/dock.rs): поле
       входит в полосы броска у краёв */
    padding: 0px;
}

.dock-split {
    border-color: transparent;
    color: transparent;
    divider-thickness: 2px;
}

.dock-group {
    background-color: var(--bg-panel);
    border-radius: 10px;
    border-width: 1px;
    border-color: var(--border-soft);
}

.dock-header {
    height: 34px;
    padding: 0px 6px;
    background-color: transparent;
    border-color: var(--border-soft);
    border-bottom-width: 1px;
}

.dock-tab {
    height: 33px;
    padding: 0px 6px 0px 10px;
    border-color: transparent;
    border-bottom-width: 2px;
    border-left-width: 2px;
    border-right-width: 2px;
}

.dock-tab:hover {
    background-color: transparent;
}

.dock-tab.active {
    background-color: transparent;
    border-bottom-color: var(--primary);
}

.dock-tab.drop-before { border-left-color: var(--primary); }
.dock-tab.drop-after { border-right-color: var(--primary); }

.dock-tab-title {
    font-size: 12px;
    font-weight: 500;
    color: var(--text-subtle);
}

.dock-tab:hover .dock-tab-title {
    color: var(--text-muted);
}

.dock-tab.active .dock-tab-title {
    color: var(--text);
}

/* крестик — только у активной вкладки и под курсором */
.dock-tab-close {
    font-size: 14px;
    color: transparent;
}

.dock-tab:hover .dock-tab-close,
.dock-tab.active .dock-tab-close {
    color: var(--text-subtle);
}

.dock-tab .dock-tab-close:hover {
    color: var(--text);
}

/* крестик «скрыть область» у пустой области документов — виден всегда */
.dock-group-close .dock-tab-close {
    color: var(--text-subtle);
}

.dock-drop-hint {
    background-color: var(--primary-soft);
    border-color: var(--primary);
    border-width: 2px;
    border-radius: 8px;
}

/* ── панель «Проект» ── */
.project-panel {
    padding: 10px 8px;
}

.section-caption {
    padding: 0px 8px 6px 8px;
    color: var(--text-subtle);
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 1px;
}

.tree-row {
    height: 28px;
    padding: 0px 8px;
    border-radius: 6px;
    align-items: center;
}

.tree-row:hover {
    background-color: var(--surface-hover);
}

.tree-row.selected {
    background-color: var(--primary-soft);
}

.tree-icon {
    icon-size: 16px;
    color: var(--text-subtle);
}

.tree-row.selected .tree-icon {
    color: var(--primary);
}

.tree-text {
    font-size: 13px;
    color: var(--text);
}

.tree-text.muted {
    color: var(--text-muted);
}

.tree-meta {
    font-size: 11px;
    color: var(--text-subtle);
}

/* ── журнал ── */
.log-panel {
    padding: 8px 12px;
}

.log-time {
    font-family: monospace;
    font-size: 12px;
    color: var(--text-subtle);
}

.log-text {
    font-family: monospace;
    font-size: 12px;
    color: var(--text-muted);
}

.log-warn {
    color: var(--warning);
}

.log-error {
    color: var(--error);
}

/* ── кнопка палитры в заголовке ── */
.palette-pill {
    width: 440px;
    height: 28px;
    padding: 0px 6px 0px 10px;
    border-radius: 8px;
    border-width: 1px;
    border-color: var(--border);
    background-color: var(--bg-field);
    cursor: pointer;
    transition: border-color 120ms ease;
}

.palette-pill:hover {
    border-color: var(--primary);
}

.palette-pill-icon {
    icon-size: 16px;
    color: var(--text-subtle);
}

.palette-pill-text {
    font-size: 12px;
    color: var(--text-subtle);
}

/* ── строка меню ── */
.menubar-item {
    height: 28px;
    padding: 0px 9px;
    border-radius: 6px;
}

.menubar-title {
    font-size: 13px;
    color: var(--text-muted);
}

.menubar-item:hover .menubar-title,
.menubar-item.open .menubar-title {
    color: var(--text);
}

/* ── панель «Структура» ── */
.structure-header {
    min-height: 40px;
    padding: 5px 10px;
    border-bottom-width: 1px;
    border-color: var(--border-soft);
}

.structure-status {
    font-size: 12px;
    color: var(--text-subtle);
}

.structure-status.ok {
    color: var(--text-muted);
}

.structure-status.error {
    color: var(--error);
}

.struct-name {
    font-size: 12px;
    color: var(--text);
}

.tree-chevron {
    icon-size: 16px;
    color: var(--text-subtle);
}

.color-dot {
    width: 8px;
    height: 8px;
    border-radius: 4px;
}

.pattern-editor {
    background-color: var(--bg-hex);
}

.search-query {
    width: 340px;
    font-size: 12px;
    padding: 5px 10px;
}

/* ── панель «Сведения» ── */
.info-panel {
    padding: 12px 14px;
}

.entropy-chart {
    height: 90px;
    padding: 6px;
    border-radius: 8px;
    border-width: 1px;
    border-color: var(--border-soft);
    background-color: var(--bg-hex);
    cursor: pointer;
}

.info-label {
    width: 96px;
}

.info-value {
    font-family: monospace;
    font-size: 12px;
    color: var(--text);
}

.btn-small {
    font-size: 12px;
    padding: 4px 12px;
}

.mono {
    font-family: monospace;
}

.bookmark-icon {
    color: var(--modified);
}

.filter-field {
    width: 100%;
    font-size: 12px;
    padding: 5px 10px;
}

ListingView {
    background-color: var(--bg-hex);
    color: #D4D9E3;
    accent-color: var(--primary);
    scrollbar-color: #FFFFFF2A;
    font-size: 13px;
}

GraphView {
    background-color: var(--bg-hex);
    color: #D4D9E3;
    border-color: #2A303C;
    accent-color: var(--primary);
    selection-color: #181C24;
    font-size: 12px;
}

/* ── консоль ── */
.console-bar {
    padding: 6px 8px;
    border-top-width: 1px;
    border-color: var(--border-soft);
}

.console-field {
    width: 100%;
    font-family: monospace;
    font-size: 12px;
    padding: 5px 10px;
}

.console-input {
    font-family: monospace;
    font-size: 12px;
    color: var(--primary);
}

.console-value {
    font-family: monospace;
    font-size: 12px;
    color: #98C379;
}

/* ── отладка ── */
.dbg-sep {
    width: 1px;
    height: 18px;
    margin: 0px 6px;
    background-color: var(--border);
}

.dbg-dot {
    width: 8px;
    height: 8px;
    margin: 0px 6px 0px 0px;
    border-radius: 4px;
    background-color: var(--text-subtle);
}

.dbg-dot.dbg-running {
    background-color: var(--success);
}

.dbg-dot.dbg-stopped {
    background-color: var(--warning);
}

.dbg-row {
    height: 24px;
}

.dbg-reg-name {
    width: 56px;
    font-size: 12px;
    color: var(--text-muted);
}

.dbg-reg-value {
    font-size: 12px;
    color: var(--text);
}

.dbg-reg-value.changed {
    color: var(--modified);
}

.dbg-reg-value.code {
    color: var(--primary-hover);
}

.dbg-bp-dot {
    width: 10px;
    height: 10px;
    border-radius: 5px;
    background-color: var(--error);
}

.dbg-bp-cond {
    background-color: var(--warning);
}

.dbg-bp-label {
    width: 84px;
}

.dbg-bp-editor {
    padding: 6px 12px 8px 32px;
}

.dbg-status {
    padding: 6px 12px;
    border-bottom-width: 1px;
    border-color: var(--border-soft);
}

.dbg-mem {
    height: 20px;
}

.terminal-area {
    padding: 4px 0px 0px 8px;
    background-color: var(--bg-hex);
}

.terminal {
    background-color: var(--bg-hex);
    color: var(--text);
}

/* ── ИИ ── */
.ai-feed {
    padding: 12px 14px;
}

.ai-empty {
    padding: 24px 6px;
}

.ai-hero-icon {
    icon-size: 34px;
    color: var(--primary);
}

.ai-hero-title {
    font-size: 18px;
    font-weight: 600;
    color: var(--text);
}

.ai-chip {
    font-size: 13px;
    padding: 8px 12px;
    border-radius: var(--radius-field);
}

.ai-user {
    max-width: 85%;
    border-radius: 12px;
    background-color: var(--primary-soft);
}

/* выпадающие списки шапки чата: ширина по выбранному, меню может быть шире поля */
.ai-dd {
    --fit-selected: true;
    --popup-min-width: 260px;
    --popup-max-width: 560px;
}

.ai-user-edit {
    padding: 8px;
    border-radius: 12px;
    border-width: 1px;
    border-color: var(--primary);
    background-color: var(--bg-panel-2);
}

.ai-msg-act {
    width: 24px;
    height: 24px;
    font-size: 14px;
    color: var(--text-muted);
    opacity: 0.55;
}

.ai-msg-act:hover {
    opacity: 1;
}

.ai-input {
    width: 100%;
    font-size: 13px;
}

/* вложения к сообщению агенту */
.ai-attach-chip {
    padding: 2px 4px 2px 8px;
    border-radius: 8px;
    background: var(--primary-soft);
    border: 1px solid var(--border-soft);
}

.ai-attach-text {
    font-size: 12px;
    color: var(--text);
}

.ai-user-text {
    padding: 8px 12px;
    font-size: 13px;
    color: var(--text);
}

.ai-md {
    font-size: 13px;
    color: var(--text);
    --md-heading-color: var(--text);
    --md-link-color: var(--primary-hover);
    --md-code-bg: #FFFFFF10;
    --md-code-color: #E5C07B;
    --md-code-font-size: 12px;
    --md-code-padding-h: 4px;
    --md-code-radius: 4px;
    --md-code-block-bg: var(--bg-hex);
    --md-code-block-color: var(--text);
    --md-code-block-radius: 8px;
    --md-code-block-padding: 10px;
    --md-quote-bg: var(--bg-panel-2);
    --md-quote-text-color: var(--text-muted);
    --md-table-border-color: var(--border-soft);
    --md-table-header-bg: var(--bg-panel-2);
    --md-table-header-color: var(--text);
    --md-table-stripe-bg: #FFFFFF08;
    --md-hr-color: var(--border-soft);
}

.ai-thinking {
    padding: 6px 10px;
    border-radius: 8px;
    border-left-width: 2px;
    border-color: var(--border);
}

.ai-thinking-text {
    font-size: 12px;
    color: var(--text-subtle);
}

.ai-tool {
    padding: 5px 10px;
    border-radius: 8px;
    background-color: var(--bg-panel-2);
}

.ai-tool-icon {
    icon-size: 14px;
    color: var(--text-subtle);
}

/* инструмент выполняется — жёлтый ключ мигает (видно, что работа идёт, а не встала) */
@keyframes ai-tool-blink { 0% { opacity: 1; } 50% { opacity: 0.25; } 100% { opacity: 1; } }
.ai-tool.running .ai-tool-icon {
    color: var(--warning);
    animation: ai-tool-blink 1000ms ease-in-out infinite;
}

/* отсчёт инструмента, работающего дольше 10 с */
.ai-tool-timer {
    font-size: 11px;
    color: var(--warning);
}

.ai-tool.fail .ai-tool-icon {
    color: var(--error);
}

.ai-tool-name {
    font-size: 12px;
    color: var(--text-muted);
}

.ai-tool-summary {
    font-size: 12px;
    color: var(--text-subtle);
}

.ai-error {
    padding: 8px 12px;
    border-radius: 8px;
    background-color: var(--warning-soft);
}

.ai-error-text {
    font-size: 12px;
    color: var(--error);
}

.ai-confirm {
    margin: 0px 10px 8px 10px;
    padding: 12px;
    border-radius: var(--radius-card);
    border-width: 1px;
    border-color: var(--warning);
    background-color: var(--bg-panel-2);
}

.ai-confirm-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
}

.ai-confirm-summary {
    font-size: 12px;
    color: var(--text-muted);
}

.toggle-on {
    color: var(--primary);
}

/* ── определение ── */
.det-body {
    padding: 10px 12px;
}

.det-card {
    padding: 10px 12px;
    border-radius: var(--radius-card);
    background-color: var(--bg-panel-2);
}

.det-section-icon {
    icon-size: 18px;
    color: var(--text-muted);
}

.det-section-icon.packer { color: var(--warning); }
.det-section-icon.protector { color: var(--error); }
.det-section-icon.compiler { color: var(--primary-hover); }
.det-section-icon.installer { color: var(--success); }
.det-section-icon.crypto { color: #C678DD; }

.det-section-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
}

.det-count {
    font-size: 11px;
    color: var(--text-subtle);
}

.det-item {
    padding: 6px 8px;
    border-radius: 8px;
    background-color: #FFFFFF08;
}

.det-name {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
}

.det-version {
    padding: 1px 7px;
    border-radius: var(--radius-pill);
    background-color: var(--primary-soft);
}

.det-version-text {
    font-size: 11px;
    color: var(--primary-hover);
}

.det-bar {
    width: 60px;
    height: 5px;
    border-radius: 3px;
    background-color: #FFFFFF14;
}

.det-bar-fill {
    height: 5px;
    border-radius: 3px;
}

.det-bar-fill.high { background-color: var(--success); }
.det-bar-fill.mid { background-color: var(--warning); }
.det-bar-fill.low { background-color: var(--text-subtle); }

.det-conf {
    width: 36px;
    font-size: 11px;
    color: var(--text-muted);
}

.det-details {
    font-size: 12px;
    color: var(--text-muted);
}

.det-lib-row {
    height: 30px;
}

.det-default { color: var(--text-subtle); }
.det-default.on { color: var(--warning); }

/* ── проект ── */
.files-header {
    padding: 4px 4px 0px 4px;
}

.files-root-icon {
    icon-size: 18px;
    color: var(--warning);
}

.files-root {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
}

.files-branch {
    padding: 2px 8px;
    border-radius: var(--radius-pill);
    background-color: var(--primary-soft);
}

.files-branch-icon {
    icon-size: 12px;
    color: var(--primary-hover);
}

.files-branch-text {
    font-size: 11px;
    color: var(--primary-hover);
}

.files-sum { font-size: 11px; }
.files-sum.m { color: #FFB454; }
.files-sum.u { color: #6BD36B; }
.files-sum.d { color: #EE5E48; }
.files-sum.c { color: #B07FFF; }

.files-tree {
    background-color: transparent;
    color: var(--text);
    border-color: transparent;
    accent-color: var(--primary);
    icon-color: var(--text-muted);
}

/* ── панель «Открытые» ── */
.open-row {
    padding: 4px 4px 4px 8px;
    border-radius: 6px;
}

.open-row:hover {
    background-color: var(--surface-hover);
}

.open-row.selected {
    background-color: var(--primary-soft);
}

.open-row.selected .tree-icon {
    color: var(--primary);
}

.open-badge {
    padding: 0px 6px;
    border-radius: 8px;
    background-color: var(--surface-hover);
}

.open-badge-text {
    font-size: 10px;
    color: var(--text-muted);
}

.files-open {
    padding: 6px 0px 0px 0px;
    border-top-width: 1px;
    border-color: var(--border-soft);
}

.term-tabs {
    padding: 4px 8px;
    border-bottom-width: 1px;
    border-color: var(--border-soft);
}

.term-tab {
    padding: 2px 4px 2px 10px;
    border-radius: 8px;
}

.term-tab:hover { background-color: var(--surface-hover); }
.term-tab.on { background-color: var(--primary-soft); }

.term-tab-icon {
    icon-size: 14px;
    color: var(--text-muted);
}

.term-tab-text {
    font-size: 12px;
    color: var(--text);
}

.script-dirty {
    font-size: 12px;
    color: var(--warning);
}

/* ── страница «Библиотека скриптов» ── */
.lib-side {
    background-color: var(--bg-panel);
    border-radius: 12px;
}

.lib-side-head {
    padding: 14px 12px 10px 14px;
}

.lib-title {
    font-size: 15px;
    font-weight: 600;
    color: var(--text);
}

.lib-tree {
    padding: 0px 6px 6px 6px;
}

.lib-side-foot {
    padding: 8px 14px;
    border-top: 1px solid var(--border);
}

.lib-main {
    background-color: var(--bg-panel);
    border-radius: 12px;
}

.lib-output {
    border-top: 1px solid var(--border);
}

.lib-welcome {
    width: 460px;
    padding: 28px;
}

.lib-welcome-icon {
    icon-size: 48px;
    color: var(--primary-hover);
}

.lib-welcome-title {
    font-size: 18px;
    font-weight: 600;
    color: var(--text);
}

/* ── страница «Настройки» ── */
.set-nav {
    width: 230px;
    padding: 14px 10px;
    background-color: var(--bg-panel);
    border-radius: 12px;
}

.set-nav-title {
    padding: 0px 10px 10px 10px;
}

.set-nav-item {
    padding: 8px 10px;
    border-radius: 8px;
}

.set-nav-item:hover { background-color: var(--surface-hover); }
.set-nav-item.on { background-color: var(--primary-soft); }

.set-nav-icon {
    icon-size: 18px;
    color: var(--text-muted);
}

.set-nav-item.on .set-nav-icon { color: var(--primary-hover); }

.set-nav-text {
    font-size: 13px;
    color: var(--text);
}

.set-main {
    padding: 0px 0px 0px 8px;
}

.set-body {
    padding: 18px 28px 28px 20px;
}

/* боковая колонка раздела настроек (каталоги скилов, библиотека промптов) */
.set-side {
    width: 400px;
}

/* плитка скила: переключатель, имя, описание в 3 строки, правка */
.set-skill {
    padding: 10px 8px 10px 12px;
    border-radius: 10px;
    background-color: var(--bg-panel-2);
    border: 1px solid var(--border);
    height: 92px;
}

.set-skill:hover {
    border-color: var(--border-strong, var(--border));
}

.set-skill.off {
    opacity: 0.55;
}

.set-title {
    font-size: 22px;
    font-weight: 600;
    color: var(--text);
}

.set-card {
    padding: 16px 18px;
    background-color: var(--bg-panel-2);
    border: 1px solid var(--border);
    border-radius: 12px;
}

.set-card-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--text);
}

.set-label {
    font-size: 13px;
    color: var(--text);
}

.set-dir {
    padding: 6px 6px 6px 8px;
    border-radius: 8px;
    background-color: var(--surface-hover);
}

/* ── панель ИИ: шапка с режимом и строка статуса ── */
.ai-head {
    padding: 8px 10px;
    border-bottom-width: 1px;
    border-color: var(--border-soft);
}

.ai-status {
    padding: 6px 12px 4px 12px;
    border-top-width: 1px;
    border-color: var(--border-soft);
}

.ai-status-text {
    font-size: 11px;
    color: var(--text-subtle);
}

.ai-ctx-bar {
    height: 4px;
    color: var(--primary);
}

.ai-ctx-bar.full {
    color: var(--warning);
}

/* настройки: бейджи инструментов, выключенное */
.set-badge {
    padding: 1px 7px;
    border-radius: 999px;
}

.set-badge.read { background-color: var(--success-soft); }
.set-badge.write { background-color: var(--warning-soft); }

.set-badge-text {
    font-size: 10px;
    color: var(--text-muted);
}

.set-dir.off {
    opacity: 0.55;
}

.dim {
    color: var(--text-subtle);
}

/* ── окно вывода инструмента ИИ: тёмный терминал ── */
.ai-term {
    border-radius: 10px;
    background-color: #0B0E14;
    border: 1px solid #FFFFFF14;
}

.ai-term-bar {
    padding: 6px 10px;
    border-radius: 10px 10px 0px 0px;
    background-color: #151A23;
    border-bottom-width: 1px;
    border-color: #FFFFFF0F;
}

.ai-term-dot {
    width: 9px;
    height: 9px;
    border-radius: 5px;
    background-color: #3A404C;
}

.ai-term-dot.r { background-color: #FF5F57; }
.ai-term-dot.y { background-color: #FEBC2E; }
.ai-term-dot.g { background-color: #28C840; }

.ai-term-cmd {
    font-family: monospace;
    font-size: 11px;
    color: #8B949E;
}

.ai-term-state {
    font-size: 10px;
    color: #6E7681;
}

.ai-term-state.run { color: #3FB950; }

.ai-term-body {
    padding: 8px 12px 10px 12px;
}

.ai-term-body.pad {
    padding: 0px;
}

.ai-term-out {
    padding: 8px 12px 10px 12px;
}

.ai-term-more.pad,
.ai-term-wait.pad {
    padding: 8px 12px 0px 12px;
}

.ai-term-line {
    font-family: monospace;
    font-size: 11px;
    line-height: 1.45;
    color: #C9D1D9;
}

.ai-term-line.err {
    color: #FF7B72;
}

.ai-term-more,
.ai-term-wait {
    font-family: monospace;
    font-size: 11px;
    color: #6E7681;
}

.ai-term-cursor {
    font-family: monospace;
    font-size: 11px;
    color: #3FB950;
}

/* строка инструмента: число строк и стрелка раскрытия */
.ai-tool:hover { background-color: var(--surface-hover); }

.ai-tool-count {
    font-size: 11px;
    color: var(--text-subtle);
}

.ai-tool-chevron {
    icon-size: 16px;
    color: var(--text-subtle);
}

/* ── вкладки чатов ИИ ── */
.ai-chat-tab {
    padding: 3px 8px;
    border-radius: 8px;
}

.ai-chat-tab:hover { background-color: var(--surface-hover); }
.ai-chat-tab.on { background-color: var(--primary-soft); }

.ai-chat-title {
    font-size: 12px;
    color: var(--text-muted);
}

.ai-chat-tab.on .ai-chat-title { color: var(--text); }

.ai-chat-icon {
    icon-size: 12px;
    color: var(--text-subtle);
}

.ai-chat-close {
    icon-size: 13px;
    color: var(--text-subtle);
}

.ai-chat-close:hover { color: var(--error); }

.ai-chat-busy {
    width: 7px;
    height: 7px;
    border-radius: 4px;
    background-color: var(--success);
}

.ai-bind.on { color: var(--primary-hover); }
.set-expand { color: var(--text-subtle); }
.set-expand.on { color: var(--success); }

/* настройки: редактор и предпросмотр промпта */
.set-prompt-editor {
    height: 320px;
    border-radius: 8px;
    border: 1px solid var(--border);
}

.set-preview {
    padding: 10px 12px;
    border-radius: 8px;
    background-color: var(--bg-hex);
}

.set-preview-text {
    font-family: monospace;
    font-size: 11px;
    color: var(--text-muted);
}

.set-dir.on {
    background-color: var(--primary-soft);
}

.on-accent {
    color: var(--primary-hover);
}

/* настройки терминала */
.term-swatches {
    width: 269px;
    height: 14px;
}

.set-wide {
    width: 260px;
}

.set-num {
    font-size: 13px;
    color: var(--text);
    min-width: 28px;
    text-align: center;
}

/* ── панели плагинов Rhai ── */
.plugin-text {
    font-size: 13px;
    color: var(--text);
}

.plugin-code {
    padding: 8px 10px;
    border-radius: 8px;
    background-color: var(--bg-hex);
}

.plugin-divider {
    height: 1px;
    background-color: var(--border);
}

/* TreeView в панели плагина (высота — в коде: 300px или .height() плагина) */
.plugin-tree {
    background-color: transparent;
    color: var(--text);
    border-color: transparent;
    accent-color: var(--primary);
    icon-color: var(--text-muted);
}

.plugin-table {
    height: 280px;
}

.plugin-foot {
    padding: 4px 8px 4px 12px;
    border-top: 1px solid var(--border-soft);
}

/* панель «Типы» */
.types-status {
    padding: 6px 10px;
    border-top: 1px solid var(--border-soft);
}

.types-detail {
    border-top: 1px solid var(--border-soft);
}

.set-indent {
    padding: 0px 0px 0px 22px;
}


/* список выбора в диалоге скрипта (choose, pick_function) */
ListView.choice-list {
    background-color: var(--bg-panel-2);
    color: var(--text);
    border-color: var(--border);
    accent-color: var(--primary);
    font-size: 12px;
}

.btn-danger {
    background-color: var(--error);
    color: var(--on-primary);
}

/* ── История (недавние файлы) ── */
.history-body {
    padding: 18px 28px 28px 20px;
    max-width: 1200px;
}

.history-day {
    padding: 14px 8px 4px 8px;
    color: var(--text-subtle);
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 1px;
}

.history-row {
    height: 46px;
    padding: 0px 10px;
    border-radius: 8px;
    align-items: center;
}

.history-row:hover {
    background-color: var(--surface-hover);
}

.history-slot {
    width: 28px;
}

.history-search {
    width: 320px;
}

/* ── Редактор кода во вкладке документа ── */
.code-head {
    padding: 4px 10px;
    border-bottom: 1px solid var(--border);
}

/* ── Проблемы (LSP) ── */
.problem-error {
    icon-size: 14px;
    color: #f87171;
}

.problem-warn {
    icon-size: 14px;
    color: #fbbf24;
}

.problem-info {
    icon-size: 14px;
    color: #60a5fa;
}

/* ── Менеджер плагинов ── */
.plugins-window {
    background-color: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: 14px;
    padding: 18px 20px;
    height: 720px;
}

.plugins-side {
    width: 230px;
}

/* тело окна занимает остаток высоты: сетка плиток прокручивается внутри окна */
.plugins-body {
    flex-grow: 1;
}

.plugin-head-icon {
    icon-size: 22px;
    color: var(--primary);
}

.plugin-tile {
    background-color: var(--bg-panel-2);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 14px;
    height: 168px;
}

.plugin-tile:hover {
    border-color: var(--primary);
}

.plugin-tile-icon-wrap {
    background-color: var(--primary-soft);
    border-radius: 10px;
    padding: 9px;
}

.plugin-tile-icon {
    icon-size: 20px;
    color: var(--primary);
}

.plugin-tile-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--text);
}

.plugin-tile-desc {
    height: 52px;
}

/* поиск в файлах: поле маски */
.search-glob {
    width: 170px;
}

/* ── Окна плагинов (window(...) из ui()) ── */
.plugin-window {
    background-color: var(--bg-panel);
    border: 1px solid var(--border);
    border-radius: 14px;
    padding: 18px 20px;
}

/* биты числа (bitfield): подпись над флажком */
.bit-cell {
    padding: 2px 3px;
    border-radius: 6px;
    background-color: var(--bg-panel-2);
}

.bit-off {
    opacity: 0.45;
}

.bit-label {
    font-size: 10px;
    color: var(--text-muted);
}

/* плитки списка с поиском (search_list(...).tiles(N)) */
.pick-tile {
    background-color: var(--bg-panel-2);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 10px 12px;
}
.pick-tile:hover {
    border-color: var(--primary);
}
.pick-tile.selected {
    border-color: var(--primary);
    background-color: var(--primary-soft);
}
.pick-tile-icon-wrap {
    background-color: var(--primary-soft);
    border-radius: 8px;
    padding: 6px;
}
.pick-tile-icon {
    icon-size: 16px;
    color: var(--primary);
}
.pick-tile-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
}
.pick-tile-tag {
    background-color: var(--surface-hover);
    border-radius: 6px;
    padding: 1px 7px;
    font-size: 11px;
    color: var(--text-muted);
}

/* ── панель «Сборка» ── */
OptionGroup {
    color: var(--text);
    accent-color: var(--primary);
    border-color: var(--text-subtle);
    font-size: 13px;
}

.build-ok {
    font-size: 16px;
    color: var(--success);
}

.build-miss {
    font-size: 16px;
    color: var(--error);
}

.build-opt {
    font-size: 16px;
    color: var(--warning);
}

.build-req {
    background-color: var(--bg-field);
    border-radius: 6px;
}

.build-log {
    font-family: monospace;
    font-size: 12px;
    color: var(--text-muted);
}

.build-log-box {
    height: 320px;
    margin: 0 12px 12px 12px;
    background-color: var(--bg-field);
    border-radius: 6px;
}

/* панель «Весь код»: ход декомпиляции программы */
.decomp-all-progress {
    width: 360px;
}

/* панель «Весь код»: дерево проекта */
.code-tree-side {
    padding: 6px 4px 0px 6px;
    border-right-width: 1px;
    border-color: var(--border);
}

.code-tree-head {
    padding: 2px 4px 4px 4px;
}

/* мастер выгрузки проекта исходников */
.wizard {
    padding: 18px 20px;
}

.wizard-icon-wrap {
    width: 38px;
    height: 38px;
    border-radius: 10px;
    background-color: var(--primary-soft, #4F8CFF26);
    align-items: center;
    justify-content: center;
}

.wizard-icon {
    font-size: 22px;
    color: var(--primary, #4F8CFF);
}

.wizard-step {
    padding: 5px 12px 5px 6px;
    border-radius: 16px;
    border-width: 1px;
    border-color: var(--border);
}

.wizard-step.active {
    border-color: var(--primary, #4F8CFF);
    background-color: var(--primary-soft, #4F8CFF1F);
}

.wizard-step-num {
    width: 22px;
    height: 22px;
    border-radius: 11px;
    background-color: var(--surface-2, #FFFFFF14);
    align-items: center;
    justify-content: center;
}

.wizard-step.active .wizard-step-num {
    background-color: var(--primary, #4F8CFF);
}

.wizard-step.done .wizard-step-num {
    background-color: var(--success, #3FB950);
}

.wizard-step-num-text {
    font-size: 11px;
    font-weight: 600;
    color: var(--text);
}

.wizard-step-text {
    font-size: 13px;
    color: var(--text-muted);
}

.wizard-step.active .wizard-step-text {
    color: var(--text);
    font-weight: 600;
}

.wizard-step-line {
    width: 28px;
    height: 1px;
    background-color: var(--border);
}

.wizard-body {
    height: 440px;
    padding-top: 4px;
}

.wizard-col {
    width: 360px;
}

.wizard-section {
    font-size: 10.5px;
    font-weight: 600;
    letter-spacing: 0.6px;
    color: var(--text-subtle, #5D6779);
}

.wizard-preview {
    flex-grow: 1;
    padding: 6px;
    border-radius: 8px;
    border-width: 1px;
    border-color: var(--border);
    background-color: var(--bg-field);
}

.wizard-key {
    width: 110px;
}

.plugin-table-fill {
    min-height: 160px;
}

.plugin-image {
    height: 180px;
    width: 320px;
}

/* сообщение в очереди: написано во время хода, войдёт после ближайшего шага модели */
.ai-queued {
    padding: 4px 4px 0 8px;
    border-width: 1px;
    border-style: dashed;
    border-color: var(--primary);
    opacity: 0.8;
}

/* индикатор хода в панели «Сборка» */
.build-progress {
    padding: 2px 0px 0px 0px;
}

/* низ панели «Сборка»: ход этапов во всю ширину */
.build-footer {
    border-top: 1px solid var(--border-soft);
    padding: 8px 8px 4px 8px;
}

/* полосы хода: заливка — основной цвет, процент — приглушённым текстом (без правила заливка и процент
   брали унаследованный цвет и были не видны: «прогрессбар так и не заполняется») */
ProgressBar {
    background-color: var(--border-soft);
    accent-color: var(--primary);
    color: var(--text-muted);
    font-size: 11px;
}

/* ход сборки в панели «Сборка»: Выгрузка → Сборка → Готово */
Stepper.status {
    color: var(--text);
    accent-color: var(--primary);
    border-color: var(--border-soft);
    font-size: 12px;
    icon-size: 14px;
    gap: 24px;
    border-width: 2px;
}

/* ── страница «Библиотека»: разделы и тулчейны ── */
.lib-nav {
    width: 220px;
}

.tc-body {
    padding: 22px 28px 28px 24px;
}

.tc-field-label {
    width: 190px;
}

.tc-progress {
    width: 320px;
}

.tc-stat {
    width: 150px;
    padding: 12px 14px;
}

.tc-stat-value {
    font-size: 22px;
    font-weight: 700;
    color: var(--text);
}

.tc-error {
    padding: 10px 14px;
    border-radius: 10px;
    background-color: var(--danger-soft, var(--surface-hover));
}

.tc-error-text {
    font-size: 13px;
    color: var(--danger, var(--text));
}

.badge-warn { background-color: var(--warning-soft, var(--surface-hover)); }
.badge-warn .badge-text { color: var(--warning); }

.tc-row.on { background-color: var(--primary-soft); }
