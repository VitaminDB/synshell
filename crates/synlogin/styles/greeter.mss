/* Экран входа synlogin. */
.bg { background: linear-gradient(160deg, var(--accent) 0%, var(--bg) 55%, #000000 100%); }
.root { padding: 64px 6px 28px 6px; }
.time { font-size: 72px; font-weight: 200; color: #ffffff; text-shadow: 0px 2px 12px #00000070; }
.date { font-size: 16px; color: #ffffffd0; }
.title { font-size: 22px; font-weight: 600; color: #ffffff; }
.users-row { padding: 6px; }
.user-card {
    width: 116px;
    padding: 16px 8px;
    border-radius: 24px;
    background-color: #ffffff18;
    transition: background-color 150ms ease-out, scale 220ms spring(420, 26);
}
.user-card:hover { background-color: #ffffff28; }
.user-card:active { scale: 0.95; }
.avatar { width: 72px; height: 72px; border-radius: 36px; background-color: var(--accent); }
.avatar.big { width: 96px; height: 96px; border-radius: 48px; }
.avatar-letter { font-size: 32px; font-weight: 600; color: #ffffff; }
.user-name { font-size: 14px; color: #ffffff; }
.field-box { width: 320px; }
.field { border-radius: 14px; }
.error { font-size: 13px; color: #ffb4a9; }
.hint { font-size: 13px; color: #ffffffc8; }
.pill { padding: 10px 20px; border-radius: 999px; background-color: #ffffff22; transition: background-color 150ms ease-out, scale 200ms spring(420, 26); }
.pill:active { scale: 0.95; }
.pill-primary { background-color: var(--accent); }
.pill-text { font-size: 14px; color: #ffffff; }
.pill-icon { font-size: 20px; color: #ffffff; }
.round { padding: 12px; border-radius: 999px; background-color: #ffffff1c; transition: scale 200ms spring(420, 26); }
.round:active { scale: 0.9; }
.round-icon { font-size: 22px; color: #ffffff; }
.osk { padding: 6px 4px; border-radius: 18px; background-color: #000000a8; }
.osk-key {
    border-radius: 8px;
    background-color: #ffffff22;
    transition: background-color 100ms ease-out;
}
.osk-key:active { background-color: var(--accent); }
.osk-key-action { background-color: #ffffff12; }
.osk-key-on { background-color: var(--accent); }
.osk-key-text { font-size: 18px; color: #ffffff; }
.osk-key-action .osk-key-text { font-size: 13px; }
