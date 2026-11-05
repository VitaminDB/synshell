/* Экран входа на E Ink (поверх greeter.mss): белый фон без градиента, чёрный текст, карточки и
   кнопки — белые в чёрной рамке, без полупрозрачности и теней. В быстрых режимах панели
   (ultrafast, x — только ч/б) тёмный градиент становился почти чёрным экраном. */
.bg { background: #ffffff; }
.time { color: #000000; font-weight: 300; text-shadow: none; }
.date { color: #000000; }
.title { color: #000000; }
.user-card { background-color: #ffffff; border: 2px solid #000000; }
.user-card:hover { background-color: #ffffff; }
.user-card:active { background-color: #000000; }
.avatar { background-color: #000000; }
.avatar-letter { color: #ffffff; }
.user-name { color: #000000; }
.error { color: #000000; font-weight: 600; }
.hint { color: #000000; }
.pill { background-color: #ffffff; border: 2px solid #000000; }
.pill-primary { background-color: #000000; }
.pill-text { color: #000000; }
.pill-primary .pill-text { color: #ffffff; }
.pill-icon { color: #000000; }
.round { background-color: #ffffff; border: 2px solid #000000; }
.round-icon { color: #000000; }
.zoom-box { background-color: #ffffff; border: 2px solid #000000; }
.zoom-value { color: #000000; }
.field { border: 2px solid #000000; background-color: #ffffff; color: #000000; }
.osk { background-color: #ffffff; border: 2px solid #000000; }
.osk-key { background-color: #ffffff; border: 1px solid #000000; }
.osk-key:active { background-color: #000000; }
.osk-key-action { background-color: #e0e0e0; }
.osk-key-on { background-color: #000000; }
.osk-key-text { color: #000000; }
.osk-key-on .osk-key-text { color: #ffffff; }
