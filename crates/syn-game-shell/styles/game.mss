/* syngame-shell (gameshell) — стили поверх общего shell.mss (synshell-ui).
 * Тёмная «консольная» тема независимо от светлой/тёмной темы рабочего
 * стола; акцент выделения — --accent из [appearance] (или [gameshell] accent).
 * Свои правки — ~/.config/synshell/game.mss. */

:root {
    --gs-bg: #0b0e14;
    --gs-fg: #f2f4f8;
    --gs-muted: #a3acbd;
    --gs-dim: #6b7487;
    --gs-card: #171b24;
    --gs-card-hi: #222838;
    --gs-line: #ffffff1c;
    --gs-glass: #10141dd9;
    --gs-radius: 14px;
}

.grow { flex-grow: 1; }
.gs-none { width: 0px; height: 0px; }

/* ─── Фон ────────────────────────────────────────────────────────────── */

.gs-bg-color { background-color: var(--gs-bg); }
.gs-bg-art { opacity: 0.55; }
/* Затемнение: снизу и слева плотнее — под текст афиши и полки. */
.gs-scrim {
    background: linear-gradient(180deg, #0b0e1499 0%, #0b0e1440 28%, #0b0e14d0 58%, #0b0e14f5 100%);
}

/* ─── Верхняя строка ─────────────────────────────────────────────────── */

.gs-top { padding-top: 22px; padding-bottom: 10px; }

.gs-tab {
    padding: 9px 18px;
    border-radius: 999px;
    background-color: #00000000;
    transition: background-color 160ms ease-out;
}
.gs-tab:hover { background-color: #ffffff14; }
.gs-tab-on { background-color: #ffffff22; }
.gs-tab-focus { background-color: var(--accent); }
.gs-menu-btn { padding: 9px 12px; margin-right: 6px; background-color: #ffffff14; }
.gs-tab-icon { icon-size: 20px; icon-color: var(--gs-muted); }
.gs-tab-text { font-size: 17px; font-weight: 600; color: var(--gs-muted); }
.gs-tab-on .gs-tab-icon { icon-color: var(--gs-fg); }
.gs-tab-on .gs-tab-text { color: var(--gs-fg); }
.gs-tab-focus .gs-tab-icon { icon-color: var(--accent-fg); }
.gs-tab-focus .gs-tab-text { color: var(--accent-fg); }

.gs-status-icon { icon-size: 22px; icon-color: var(--gs-fg); }
.gs-status-text { font-size: 15px; color: var(--gs-fg); }
.gs-clock { font-size: 20px; font-weight: 600; color: var(--gs-fg); }

/* Клавиши и кнопки геймпада в подсказках. */
.gs-key {
    padding: 3px 9px;
    border-radius: 7px;
    border-width: 1px;
    border-color: #ffffff40;
    background-color: #ffffff12;
}
.gs-key-text { font-size: 13px; font-weight: 700; color: var(--gs-fg); }
.gs-pad {
    width: 28px; height: 28px;
    border-radius: 14px;
    background-color: #ffffff22;
}
.gs-pad-text { font-size: 14px; font-weight: 800; color: #ffffff; text-align: center; padding-top: 4px; }
.gs-pad-a { background-color: #3f9b3a; }
.gs-pad-b { background-color: #c8413b; }
.gs-pad-x { background-color: #2f6fd1; }
.gs-pad-y { background-color: #d5a21c; }
.gs-pad-menu { background-color: #ffffff30; }

.gs-hints { padding-top: 12px; padding-bottom: 18px; }
.gs-hints-off { height: 18px; }
.gs-hint-text { font-size: 15px; color: var(--gs-muted); }

/* ─── Афиша ──────────────────────────────────────────────────────────── */

.gs-hero { padding-bottom: 4px; }
.gs-hero-title { font-size: 46px; font-weight: 800; color: var(--gs-fg); text-shadow: 0px 3px 14px #000000a0; }
.gs-hero-logo-box { height: 120px; width: 460px; }
.gs-hero-logo { height: 120px; width: 460px; }
.gs-hero-meta { font-size: 16px; color: var(--gs-muted); }

.gs-chip { padding: 5px 12px; border-radius: 999px; background-color: #ffffff1a; }
.gs-chip-text { font-size: 14px; font-weight: 600; color: var(--gs-fg); }
.gs-chip-icon { icon-size: 16px; icon-color: var(--gs-fg); }
.gs-chip-live { background-color: #3f9b3a; }

.gs-btn {
    padding: 12px 26px;
    border-radius: 999px;
    background-color: #ffffff1f;
    transition: background-color 140ms ease-out;
}
.gs-btn:hover { background-color: #ffffff33; }
.gs-btn-primary { background-color: var(--gs-fg); }
.gs-btn-primary:hover { background-color: #ffffff; }
.gs-btn-primary .gs-btn-icon { icon-color: #0b0e14; }
.gs-btn-primary .gs-btn-text { color: #0b0e14; }
.gs-btn-round { padding: 12px; }
.gs-btn-on .gs-btn-icon { icon-color: #f2c94c; }
.gs-btn-sm { padding: 8px 14px; }
.gs-btn-sm .gs-btn-text { font-size: 15px; }
.gs-btn-sm .gs-btn-icon { icon-size: 20px; }
.gs-btn-danger { background-color: #c8413b; }
.gs-btn-danger:hover { background-color: #e0524b; }
.gs-btn-icon { icon-size: 24px; icon-color: var(--gs-fg); }
.gs-btn-text { font-size: 18px; font-weight: 700; color: var(--gs-fg); }

/* ─── Полки и сетки ──────────────────────────────────────────────────── */

.gs-body { padding-top: 8px; padding-bottom: 28px; }
.gs-section-title { font-size: 20px; font-weight: 700; color: var(--gs-fg); }
.gs-shelf-row { padding-top: 10px; padding-bottom: 10px; }
.gs-grid { padding-top: 14px; }

.gs-tile {
    border-radius: var(--gs-radius);
    background-color: var(--gs-card);
    border-width: 3px;
    border-color: #00000000;
    box-shadow: 0px 6px 18px #00000070;
    transition: border-color 140ms ease-out, box-shadow 160ms ease-out, transform 160ms spring(520, 30);
}
.gs-tile-sel {
    border-color: var(--accent);
    box-shadow: 0px 0px 0px 2px #ffffff30, 0px 12px 34px #000000b0;
    transform: scale(1.06);
}
.gs-cover { border-radius: 11px; }
.gs-tile-name { font-size: 15px; color: var(--gs-muted); text-align: center; }
.gs-tile-name-sel { color: var(--gs-fg); font-weight: 700; }
.gs-tile-launch { background-color: #000000a0; border-radius: 11px; }
.gs-spinner { accent-color: var(--gs-fg); }

/* Плитки без обложки: значок на градиенте (цвет — по имени). */
.gs-fb { border-radius: 11px; padding: 14px; }
.gs-fb-name { font-size: 16px; font-weight: 700; color: #ffffff; text-align: center; }
.gs-fb-icon-glyph { icon-size: 64px; icon-color: #ffffffd0; }
.gs-fb-0 { background: linear-gradient(160deg, #3b4cca, #1b1f5e); }
.gs-fb-1 { background: linear-gradient(160deg, #c2410c, #5b1a07); }
.gs-fb-2 { background: linear-gradient(160deg, #0f766e, #083b37); }
.gs-fb-3 { background: linear-gradient(160deg, #7e22ce, #36105a); }
.gs-fb-4 { background: linear-gradient(160deg, #be123c, #520a1c); }
.gs-fb-5 { background: linear-gradient(160deg, #0369a1, #062d45); }
.gs-fb-6 { background: linear-gradient(160deg, #4d7c0f, #213607); }
.gs-fb-7 { background: linear-gradient(160deg, #a16207, #452a04); }

.gs-badge { margin: 8px; padding: 5px; border-radius: 999px; background-color: #000000a0; }
.gs-badge-icon { icon-size: 18px; icon-color: #f2c94c; }

/* ─── Пустая вкладка ─────────────────────────────────────────────────── */

.gs-empty { padding: 40px; }
.gs-empty-icon { icon-size: 96px; icon-color: var(--gs-dim); }
.gs-empty-title { font-size: 26px; font-weight: 700; color: var(--gs-fg); }
.gs-empty-text { font-size: 17px; color: var(--gs-muted); text-align: center; }

/* ─── Окна поверх: меню действий, подтверждение ──────────────────────── */

.gs-modal-scrim { background-color: #000000a8; }
.gs-modal {
    width: 460px;
    padding: 24px;
    border-radius: 20px;
    background-color: var(--gs-glass);
    backdrop-filter: blur(18px);
    border-width: 1px;
    border-color: var(--gs-line);
    box-shadow: 0px 24px 60px #000000c0;
}
.gs-modal-title { font-size: 22px; font-weight: 800; color: var(--gs-fg); padding-bottom: 10px; }
.gs-modal-text { font-size: 16px; color: var(--gs-muted); }

.gs-item {
    padding: 13px 16px;
    border-radius: 12px;
    background-color: #00000000;
    transition: background-color 120ms ease-out;
}
.gs-item-sel { background-color: var(--accent); }
.gs-item-icon { icon-size: 24px; icon-color: var(--gs-fg); }
.gs-item-text { font-size: 18px; font-weight: 600; color: var(--gs-fg); }
.gs-item-sel .gs-item-icon { icon-color: var(--accent-fg); }
.gs-item-sel .gs-item-text { color: var(--accent-fg); }

/* ─── Заставка запуска ───────────────────────────────────────────────── */

.gs-splash-bg { background-color: var(--gs-bg); }
.gs-splash-art { opacity: 0.45; }
.gs-splash-scrim { background: radial-gradient(ellipse at center, #0b0e1460, #0b0e14f0); }
.gs-splash-icon { width: 112px; height: 112px; }
.gs-splash-icon-glyph { icon-size: 112px; icon-color: var(--gs-fg); }
.gs-splash-title { font-size: 34px; font-weight: 800; color: var(--gs-fg); }
.gs-splash-text { font-size: 18px; color: var(--gs-muted); }

/* ─── Быстрое меню поверх игры ───────────────────────────────────────── */

.gs-ov-scrim { background-color: #00000080; }
.gs-ov-panel {
    width: 440px;
    padding: 26px 18px;
    background-color: #0d1118f7;
    border-right-width: 1px;
    border-color: var(--gs-line);
    box-shadow: 12px 0px 48px #000000b0;
}
.gs-ov-title { font-size: 22px; font-weight: 800; color: var(--gs-fg); padding-left: 8px; }
.gs-ov-clock { font-size: 18px; font-weight: 600; color: var(--gs-muted); padding-right: 8px; }
.gs-ov-sep { height: 1px; background-color: var(--gs-line); margin: 12px 0px 10px 0px; }
.gs-ov-meter { width: 110px; }
.gs-ov-value { font-size: 15px; color: var(--gs-muted); width: 46px; text-align: right; }
.gs-item-sel .gs-ov-value { color: var(--accent-fg); }
.gs-ov-panel .meter { height: 6px; border-radius: 3px; }
.gs-ov-panel .meter-fill { background-color: var(--gs-fg); border-radius: 3px; }
.gs-ov-panel .meter-rest { background-color: #ffffff30; border-radius: 3px; }
.gs-item-sel .meter-fill { background-color: var(--accent-fg); }
