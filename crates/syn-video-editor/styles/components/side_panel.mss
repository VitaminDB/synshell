.side-panel {
    width: 380px;
    background-color: var(--bg-panel);
    border-left-width: 1px;
    border-color: var(--border-soft);
}

.side-header {
    padding: 12px 14px 10px 14px;
}

.side-body {
    padding: 4px 14px 16px 14px;
}

.section-title {
    color: var(--text-subtle);
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 1px;
}

.panel-title {
    font-size: 15px;
    font-weight: 600;
    color: var(--text);
}

.panel-sub {
    font-size: 12px;
    color: var(--text-muted);
}

/* ── карточки результатов (панели плагинов, CarDecode) ── */
.card {
    padding: 14px;
    border-radius: var(--radius-card);
    border-width: 1px;
    border-color: var(--border);
    background-color: var(--bg-panel-2);
}

.card-ok {
    border-color: #2ECC7155;
}

.card-err {
    border-color: var(--border-soft);
    background-color: var(--bg-panel);
}

.card-brand {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-muted);
}

.card-name {
    font-size: 14px;
    font-weight: 600;
    color: var(--text);
}

.badge {
    padding: 2px 8px;
    border-radius: var(--radius-pill);
    background-color: var(--success-soft);
}

.badge-text {
    font-size: 11px;
    color: var(--success);
}

.badge-muted { background-color: var(--surface-hover); }
.badge-muted .badge-text { color: var(--text-muted); }

.code-label {
    font-size: 12px;
    color: var(--text-muted);
}

.code-value {
    font-size: 26px;
    font-weight: 700;
    color: var(--text);
    letter-spacing: 3px;
    font-family: monospace;
}

.code-value-small {
    font-size: 18px;
    font-weight: 600;
    color: var(--text);
    letter-spacing: 2px;
    font-family: monospace;
}

.note {
    font-size: 12px;
    color: var(--text-muted);
}

.err-text {
    font-size: 12px;
    color: var(--text-subtle);
}

.algo-row {
    padding: 9px 12px;
    border-radius: var(--radius-field);
    background-color: transparent;
    transition: background-color 120ms ease-out;
}

.algo-row:hover { background-color: var(--surface-hover); }

.algo-row-active {
    background-color: var(--primary-soft);
}

.algo-row-title {
    font-size: 13px;
    color: var(--text);
}

.algo-row-sub {
    font-size: 11px;
    color: var(--text-subtle);
}

.serial-field {
    width: 100%;
    font-size: 18px;
    padding: 10px 14px;
    letter-spacing: 2px;
}

/* ── инспектор данных ── */
.insp-row {
    padding: 5px 0;
    border-bottom-width: 1px;
    border-color: var(--border-soft);
}

.insp-label {
    font-size: 12px;
    color: var(--text-muted);
    width: 130px;
}

.insp-value {
    font-size: 13px;
    color: var(--text);
    font-family: monospace;
}
