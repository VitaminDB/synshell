#!/usr/bin/env python3
"""Значок лотка средствами Qt (QSystemTrayIcon → настоящие SNI и dbusmenu
реализации Qt) — проверка совместимости лотка с Qt-программами.
Запускать только в изолированной шине (dbus-run-session)."""

import sys
from PyQt6.QtGui import QAction, QIcon
from PyQt6.QtWidgets import QApplication, QMenu, QSystemTrayIcon

app = QApplication(sys.argv)
app.setQuitOnLastWindowClosed(False)
tray = QSystemTrayIcon(QIcon.fromTheme("kdeconnect", QIcon.fromTheme("network-wireless")))
tray.setToolTip("Qt-значок")
menu = QMenu()
menu.addAction("Действие Qt").triggered.connect(lambda: print("[qt-tray] action", flush=True))
chk = QAction("Флажок Qt", menu, checkable=True, checked=True)
menu.addAction(chk)
menu.addSeparator()
sub = menu.addMenu("Подменю Qt")
sub.addAction("Внутри")
menu.addAction("Выход").triggered.connect(app.quit)
tray.setContextMenu(menu)
tray.activated.connect(lambda r: print("[qt-tray] activated", r, flush=True))
tray.show()
print("[qt-tray] показан:", QSystemTrayIcon.isSystemTrayAvailable(), flush=True)
sys.exit(app.exec())
