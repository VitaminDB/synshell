/* ── QuitSmoke · базовые стили (используют токены темы) ──────────────── */

/* Фоновый слой во всю площадь (DecoratedBox в корневом Stack). */
.app-bg {
    background: var(--bg);
}

/* Встроенные диалоги syngui (ConfirmDialog/AlertDialog) — в теме приложения. */
.dialog {
    background: var(--surface);
    color: var(--fg);
    border-color: var(--border);
    accent-color: var(--accent);
}

/* Корневые контейнеры экранов (Column) сами фон не рисуют — его даёт .app-bg. */
.screen-root, .onb-root, .craving-root {
    background: var(--bg);
}

.grow { flex-grow: 1; }

/* ── Типографика ─────────────────────────────────────────────────────── */

.screen-title {
    color: var(--fg);
    font-size: 26px;
    font-weight: 800;
    letter-spacing: 0.2px;
}

.section-title {
    color: var(--fg-muted);
    font-size: 13px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.6px;
}

/* ── Карточки ────────────────────────────────────────────────────────── */

.card {
    background: var(--surface);
    border-radius: 20px;
    padding: 16px;
    border-width: 1px;
    border-color: var(--border);
    box-shadow: 0px 8px 22px var(--shadow);
}

.card-eyebrow {
    color: var(--accent-2);
    font-size: 12px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.5px;
}

.card-title {
    color: var(--fg);
    font-size: 20px;
    font-weight: 800;
}

.card-body {
    color: var(--fg-muted);
    font-size: 14px;
    line-height: 20px;
}

.card-accent {
    background: linear-gradient(135deg, var(--accent-soft), var(--surface));
    border-color: var(--accent-soft);
}

.milestone-bar {
    height: 10px;
    accent-color: var(--accent);
    background: var(--ring-track);
    border-radius: 6px;
}

/* Календарь-хитмап выкуренных: интенсивность rose = число сигарет за день. */
.heat-cell { width: 14px; height: 14px; border-radius: 3px; }
.heat-0 { background: var(--surface-2); border-width: 1px; border-color: var(--border); }
.heat-future { background: var(--surface-2); opacity: 0.35; }
.heat-1 { background: #f3b4b1; }
.heat-2 { background: #ee8a86; }
.heat-3 { background: #e5605b; }
.heat-4 { background: #c9302c; }
.heat-legend { color: var(--fg-muted); font-size: 11px; }
.heat-wd { color: var(--fg-muted); font-size: 9px; height: 14px; width: 22px; text-align: right; }
.heat-sel { border-width: 2px; border-color: var(--fg); }
.heat-detail { padding: 6px 0px 0px 0px; }
.heat-detail-hint { color: var(--fg-muted); font-size: 12px; }
.heat-detail-date { color: var(--fg); font-size: 15px; font-weight: 700; }
.heat-detail-times { color: var(--rose); font-size: 13px; font-weight: 600; }
.heat-detail-clean { color: var(--accent-2); font-size: 13px; font-weight: 600; }
.stats-empty { color: var(--fg-muted); font-size: 14px; text-align: center; padding: 22px 0px; }

/* ── Показатели ──────────────────────────────────────────────────────── */

.stat-card {
    padding: 16px;
    border-radius: 18px;
}

.stat-icon { font-size: 26px; color: var(--accent-2); }
.tint-amber .stat-icon { color: var(--amber); }
.tint-blue .stat-icon { color: var(--blue); }
.tint-rose .stat-icon { color: var(--rose); }
.tint-green .stat-icon { color: var(--accent-2); }
.tint-vape .stat-icon { color: var(--vape); }

.stat-value {
    color: var(--fg);
    font-size: 22px;
    font-weight: 800;
}

.stat-label {
    color: var(--fg-muted);
    font-size: 12px;
    font-weight: 600;
}

.tint-green { background: linear-gradient(135deg, var(--accent-soft), var(--surface)); }
.tint-amber { background: linear-gradient(135deg, var(--amber-soft), var(--surface)); }
.tint-blue  { background: linear-gradient(135deg, var(--blue-soft), var(--surface)); }
.tint-rose  { background: linear-gradient(135deg, var(--rose-soft), var(--surface)); }
.tint-vape  { background: linear-gradient(135deg, var(--vape-soft), var(--surface)); }

/* ── Геро-кольцо ─────────────────────────────────────────────────────── */

.hero-wrap {
    padding: 8px;
}

.hero-ring {
    color: var(--accent);
    accent-color: var(--accent);
    border-color: var(--ring-track);
}

.ring-green { color: var(--accent); accent-color: var(--accent); }
.ring-amber { color: var(--amber); accent-color: var(--amber); }
.ring-vape { color: var(--vape); accent-color: var(--vape); }

.ring-top {
    color: var(--fg-muted);
    font-size: 13px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    text-align: center;
}

.ring-big {
    color: var(--fg);
    font-size: 30px;
    font-weight: 800;
    text-align: center;
}

/* Текстовая подпись в кольце (например «Можно курить») — уже таймера. */
.ring-big-sm {
    color: var(--fg);
    font-size: 22px;
    font-weight: 800;
    text-align: center;
}

.ring-bottom {
    color: var(--accent-2);
    font-size: 13px;
    font-weight: 600;
    text-align: center;
}

/* ── Кнопки ──────────────────────────────────────────────────────────── */

Button {
    border-radius: 16px;
    font-size: 16px;
    font-weight: 700;
    padding: 15px;
    border: none;
    cursor: pointer;
    transition: background 160ms ease, transform 120ms ease, box-shadow 160ms ease;
}

Button.btn-primary {
    background: #1ba85e;
    color: #ffffff;
    flex-grow: 1;
    box-shadow: 0px 8px 20px var(--shadow);
    glow: 0px 0px 16px rgba(47, 191, 113, 0.45);
}
Button.btn-primary:hover { background: #16a65c; transform: scale(1.02); }
Button.btn-primary:pressed { background: #0f9a54; transform: scale(0.98); }

Button.btn-soft {
    background: var(--surface-2);
    color: var(--accent-2);
    border-width: 1px;
    border-color: var(--border);
}
Button.btn-soft:hover { background: var(--accent-soft); }
Button.btn-soft:pressed { transform: scale(0.98); }

/* ── Нижняя навигация ────────────────────────────────────────────────── */

.nav-bar {
    height: 72px;
    background: var(--nav-bg);
    padding: 8px 6px;
    border-width: 1px;
    border-color: var(--border);
    box-shadow: 0px -6px 20px var(--shadow);
}

.nav-item {
    height: 54px;
    min-width: 62px;
    border-radius: 14px;
    padding: 0px 12px;
    cursor: pointer;
    transition: background 160ms ease;
}
.nav-item:hover { background: var(--surface-2); }

.nav-item-active {
    background: var(--accent-soft);
}

.nav-icon { font-size: 22px; text-align: center; color: var(--fg-muted); }
.nav-item-active .nav-icon { color: var(--accent-2); }

.nav-label {
    color: var(--fg-muted);
    font-size: 11px;
    font-weight: 600;
    text-align: center;
}
.nav-item-active .nav-label { color: var(--accent-2); }

/* ── Онбординг ───────────────────────────────────────────────────────── */

.onb-header {
    background: var(--surface);
    padding: 18px 22px 14px 22px;
    border-width: 1px;
    border-color: var(--border);
}

.onb-progress-label {
    color: var(--fg-muted);
    font-size: 13px;
    font-weight: 700;
}

.onb-progress {
    height: 8px;
    accent-color: var(--accent);
    background: var(--ring-track);
    border-radius: 5px;
}

.onb-step { padding: 6px 0px 24px 0px; }

.onb-hero-emoji { font-size: 64px; text-align: center; color: var(--accent); }

.onb-title {
    color: var(--fg);
    font-size: 26px;
    font-weight: 800;
    text-align: center;
}

.onb-body {
    color: var(--fg-muted);
    font-size: 15px;
    line-height: 22px;
    text-align: center;
}

.field-label {
    color: var(--fg-muted);
    font-size: 13px;
    font-weight: 600;
}

.onb-back {
    padding: 8px;
    border-radius: 12px;
    cursor: pointer;
}
.onb-back:hover { background: var(--surface-2); }
.onb-back-icon { font-size: 24px; color: var(--fg); }

.onb-picker {
    accent-color: var(--accent);
    background: var(--surface);
    color: var(--fg);
    border-color: var(--border);
}

.onb-seg {
    accent-color: var(--accent);
    background: var(--surface-2);
    color: var(--fg-muted);
    border-color: var(--border);
    height: 40px;
    border-radius: 12px;
    font-weight: 600;
}

.onb-input {
    accent-color: var(--accent);
    background: var(--surface);
    color: var(--fg);
    border-color: var(--border);
}

/* ── Таймлайн здоровья ───────────────────────────────────────────────── */

.ms-row {
    padding: 14px;
    border-radius: 18px;
    opacity: 0.72;
    transition: opacity 200ms ease;
}
.ms-row-done {
    opacity: 1.0;
    background: linear-gradient(135deg, var(--accent-soft), var(--surface));
    border-color: var(--accent-soft);
}

.ms-icon { font-size: 26px; color: var(--accent-2); }

.ms-title { color: var(--fg); font-size: 16px; font-weight: 700; }
.ms-body  { color: var(--fg-muted); font-size: 13px; line-height: 18px; }

.ms-check {
    color: var(--accent);
    font-size: 24px;
    font-weight: 800;
}

.ms-ring { color: var(--accent); accent-color: var(--accent); border-color: var(--ring-track); }

/* ── Графики ─────────────────────────────────────────────────────────── */

.chart { color: var(--fg); }

/* ── Craving / дыхание ───────────────────────────────────────────────── */

.craving-root { background: linear-gradient(180deg, var(--accent-soft), var(--bg)); }

.craving-top { padding: 10px 14px; }

.craving-center { padding: 20px 0px 30px 0px; }

.craving-title {
    color: var(--fg);
    font-size: 22px;
    font-weight: 800;
    text-align: center;
}

.craving-phase {
    color: var(--accent-2);
    font-size: 30px;
    font-weight: 800;
    text-align: center;
}

.craving-instr {
    color: var(--fg-muted);
    font-size: 15px;
    text-align: center;
    line-height: 22px;
}

/* ── Достижения ──────────────────────────────────────────────────────── */

.ach-card {
    padding: 16px;
    border-radius: 18px;
    transition: transform 160ms ease;
}
.ach-unlocked {
    background: linear-gradient(135deg, var(--accent-soft), var(--surface));
    glow: 0px 0px 14px rgba(47, 191, 113, 0.28);
}
.ach-unlocked:hover { transform: scale(1.03); }

.ach-locked {
    opacity: 0.55;
    filter: grayscale(1.0);
}

.ach-icon { font-size: 34px; text-align: center; color: var(--accent-2); }
.ach-title { color: var(--fg); font-size: 15px; font-weight: 700; text-align: center; }
.ach-desc  { color: var(--fg-muted); font-size: 12px; text-align: center; line-height: 16px; }

/* ── Настройки ───────────────────────────────────────────────────────── */

/* Карточка-секция, объединяющая строки. */
.set-card {
    background: var(--surface);
    border-radius: 18px;
    border-width: 1px;
    border-color: var(--border);
    box-shadow: 0px 2px 12px var(--shadow);
}

.set-row {
    padding: 15px 16px;
}

.set-divider {
    height: 1px;
    background: var(--border);
}

.set-ico {
    font-size: 20px;
    color: var(--accent-2);
    width: 24px;
    text-align: center;
}

.setting-label {
    color: var(--fg);
    font-size: 15px;
    font-weight: 600;
}

/* Контролы в зелёной теме приложения (вместо дефолтного синего). */
.set-toggle {
    accent-color: var(--accent);
    background: var(--surface-2);
    border-color: var(--border);
}
.set-seg {
    accent-color: var(--accent);
    background: var(--surface-2);
    color: var(--fg-muted);
    border-color: var(--border);
    height: 38px;
    font-weight: 600;
    border-radius: 12px;
}
.set-dropdown {
    accent-color: var(--accent);
    background: var(--surface-2);
    color: var(--fg);
    border-color: var(--border);
    border-radius: 12px;
}

/* Строки-действия: нейтральная (зелёная) и деструктивная (rose). */
.set-action {
    padding: 16px;
    cursor: pointer;
}
.set-action .set-ico,
.set-action .setting-label { color: var(--accent-2); }
.set-action:hover { background: var(--surface-2); }

.set-action-danger .set-ico,
.set-action-danger .setting-label { color: var(--rose); }
.set-action-danger:hover { background: var(--rose-soft); }

/* ── Vape-блок ─────────────────────────────────────────────────────────── */
.vape-block {
    margin-top: 8px;
    padding-top: 12px;
    border-top-width: 1px;
    border-top-color: var(--border);
}
.vape-block-title {
    color: var(--vape);
    font-size: 14px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    margin-bottom: 8px;
}
