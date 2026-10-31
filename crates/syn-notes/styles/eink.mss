/* E Ink: чёрное на белом, без теней и наведений (каждый кадр наведения —
 * обновление панели), рамки вместо заливок. Подключается после остальных
 * таблиц, когда экран — E Ink (synshell_common::display). */
:root {
    --text-muted:       #303030;
    --text-subtle:      #505050;
    --border:           #808080;
    --border-soft:      #A0A0A0;
    --border-strong:    #000000;
    --surface-hover:    #FFFFFF;
    --bg-window:        #FFFFFF;
    --bg-shell:         #FFFFFF;
    --bg-rail:          #FFFFFF;
    --bg-panel:         #FFFFFF;
    --bg-chat:          #FFFFFF;
    --shadow-shell:     0 0 0 rgba(0, 0, 0, 0);
    --glass-shadow:     0 0 0 rgba(0, 0, 0, 0);
}
.lib-card:hover, .lib-row:hover, .lib-btn:hover, .tb-btn:hover {
    background-color: transparent;
}
.lib-thumb {
    border: 2px solid #000000;
}
.lib-tab {
    border: 2px solid #000000;
}
.lib-thumb-doc-text {
    color: #000000;
}
