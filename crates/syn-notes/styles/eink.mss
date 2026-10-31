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
    --surface-selected: #E8E8E8;
    --primary:          #000000;
    --primary-hover:    #000000;
    --primary-soft:     #E8E8E8;
    --on-primary:       #FFFFFF;
    --bg-window:        #FFFFFF;
    --bg-shell:         #FFFFFF;
    --bg-rail:          #FFFFFF;
    --bg-panel:         #FFFFFF;
    --bg-chat:          #FFFFFF;
    --shadow-shell:     0 0 0 rgba(0, 0, 0, 0);
    --glass-shadow:     0 0 0 rgba(0, 0, 0, 0);
}
.lib-card:hover, .lib-row:hover, .lib-icon-btn:hover, .lib-back:hover, .tb-btn:hover,
.lib-nav:hover, .lib-proj:hover, .lib-quick:hover, .lib-side-add:hover, .ink-tool:hover {
    background-color: transparent;
}
.lib-cover {
    box-shadow: 0 0 0 rgba(0, 0, 0, 0);
    border: 2px solid #000000;
}
.lib-cover.folder {
    border: 2px solid #000000;
    background-color: #FFFFFF;
}
.lib-nav.active {
    background-color: #000000;
}
.lib-nav.active .lib-nav-icon, .lib-nav.active .lib-nav-text {
    color: #FFFFFF;
}
.lib-bnav-pill.active {
    background-color: #000000;
}
.lib-bnav-pill.active .lib-bnav-icon {
    color: #FFFFFF;
}
.lib-doc-text, .lib-doc-h {
    color: #000000;
}
.lib-side {
    border-right: 2px solid #000000;
}
.lib-openbar {
    border-bottom: 2px solid #000000;
}
.lib-cover.doc {
    border: 2px solid #000000;
}
