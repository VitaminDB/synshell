/* Просмотрщик картинок (synfiles --viewer). Сцена — из
   просмотрщика вложений synthos; цвета — палитра рабочего стола. */

.iv-window { background: #0b0c10; color: var(--fg); font-size: 13px; }

/* ── Заголовок ─────────────────────────────────────────────────── */

.iv-titlebar {
    height: 44px;
    background: var(--titlebar);
    padding: 0px 0px 0px 12px;
    align-items: center;
}
.iv-drag { height: 44px; align-items: center; }
.iv-title-tools { height: 44px; padding: 0px 6px 0px 0px; align-items: center; }
.iv-titlebar .window-controls { height: 44px; padding: 0px 4px 0px 4px; align-items: center; }
.iv-app-icon { color: var(--accent); font-size: 18px; icon-size: 18px; }
.iv-name { font-size: 13px; font-weight: 600; color: var(--fg); }
.iv-meta { font-size: 12px; color: var(--muted); }

.iv-title-btn {
    width: 34px;
    height: 32px;
    border-radius: 6px;
    justify-content: center;
    align-items: center;
    transition: background 100ms ease;
    &:hover { background: var(--hover); }
    &:pressed { background: var(--pressed); }
}
.iv-title-btn .icon { font-size: 18px; icon-size: 18px; }
.iv-title-btn.toggled { background: var(--accent-soft); }
.iv-title-btn.toggled .icon { color: var(--accent); }

/* ── Сцена ─────────────────────────────────────────────────────── */

.iv-stage { background: #0b0c10; overflow: hidden; }
.iv-backdrop { width: 100%; height: 100%; }
.iv-backdrop-empty { background: transparent; }

/* Затемнение поверх размытой копии: картинка должна оставаться главной;
   внутренняя тень — виньетка к краям. */
.iv-scrim {
    background: linear-gradient(to bottom, rgba(8, 9, 12, 0.42), rgba(8, 9, 12, 0.55) 60%, rgba(8, 9, 12, 0.74));
    box-shadow: inset 0 0 120px rgba(0, 0, 0, 0.45);
}

/* `color` — ползунки полос прокрутки и «Загрузка…» на тёмном фоне. */
.iv-viewport { background: transparent; color: #ffffff; }

.iv-hint { font-size: 14px; color: #d8dae0; }
.iv-placeholder-icon { font-size: 48px; icon-size: 48px; color: #d8dae0; opacity: 0.7; }

ToolButton.iv-nav {
    width: 44px;
    height: 44px;
    border-radius: 22px;
    background: var(--surface);
    border: 1px solid var(--border);
    color: var(--muted);
    icon-size: 26px;
    margin: 0px 16px;
    transition: background 120ms ease, transform 120ms ease;
    &:hover { background: var(--pressed); color: var(--fg); transform: scale(1.06); }
}

/* Панель инструментов: пилюля внизу по центру. */
.iv-toolbar {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 16px;
    padding: 5px 8px;
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.40);
}
.iv-bottom-gap { width: 1px; height: 6px; }

ToolButton.iv-tool {
    width: 34px;
    height: 34px;
    border-radius: 10px;
    background: transparent;
    color: var(--muted);
    icon-size: 20px;
    transition: background 100ms ease, color 100ms ease;
    &:hover { background: var(--hover); color: var(--fg); }
}

Button.iv-tool-text {
    height: 34px;
    min-width: 40px;
    padding: 0px 8px;
    border-radius: 10px;
    border-width: 0px;
    background: transparent;
    color: var(--muted);
    font-size: 12px;
    font-weight: 700;
    &:hover { background: var(--hover); color: var(--fg); }
}

.iv-tool-sep { width: 1px; height: 18px; margin: 0px 5px; background: var(--border); }
.iv-zoom { color: var(--fg); font-size: 12px; font-weight: 600; width: 52px; text-align: center; }
.iv-counter { color: var(--muted); font-size: 12px; padding: 0px 6px; }

/* Лента миниатюр над панелью. */
.iv-strip {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 14px;
    padding: 6px;
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.40);
}
.iv-thumb {
    width: 52px;
    height: 52px;
    border-radius: 9px;
    border: 2px solid transparent;
    background: var(--field);
    overflow: hidden;
    opacity: 0.62;
    transition: opacity 100ms ease, border-color 100ms ease;
    &:hover { opacity: 1; }
}
.iv-thumb-active { opacity: 1; border-color: var(--accent); }
.iv-thumb-img { width: 100%; height: 100%; }
.iv-thumb-icon { font-size: 24px; icon-size: 24px; color: var(--muted); }

/* Сведения справа сверху. */
.iv-info {
    width: 300px;
    margin: 16px 16px 0px 0px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 16px 18px;
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.40);
}
.iv-info-title { font-size: 15px; font-weight: 600; color: var(--fg); }
.iv-info-label { font-size: 11px; color: var(--muted); }
.iv-info-value { font-size: 13px; color: var(--fg); }

.iv-message-wrap { padding: 16px 0px 0px 0px; }
.iv-message {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 8px 14px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.40);
}
.iv-message-text { font-size: 13px; color: var(--fg); }

/* ── Телефон ─────────────────────────────────────────────────── */

.iv-pbar { padding: 6px 4px; }
.iv-pbtn {
    width: 44px;
    height: 44px;
    border-radius: 22px;
    justify-content: center;
    align-items: center;
    transition: background 120ms ease;
    &:pressed { background: var(--pressed); }
}
.iv-pbtn .icon { font-size: 22px; icon-size: 22px; color: var(--fg); }
.iv-pbtn.toggled { background: var(--accent-soft); }
.iv-pbtn.toggled .icon { color: var(--accent); }
.phone .iv-name { font-size: 15px; }
.phone ToolButton.iv-tool { width: 40px; height: 40px; }
