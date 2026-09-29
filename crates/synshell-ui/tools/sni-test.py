#!/usr/bin/env python3
"""Тестовый значок StatusNotifierItem с меню dbusmenu — для проверки лотка.

Запуск (только в изолированной шине!):
    dbus-run-session -- sh -c 'syndesktop-shell & sleep 2; python3 sni-test.py'

Аргументы: --path-register (регистрироваться путём объекта, как
libappindicator/Electron), --pixmap (значок пикселями вместо имени),
--passive (статус Passive), --name NAME (имя значка).
"""

import sys
import dbus
import dbus.service
from dbus.mainloop.glib import DBusGMainLoop
from gi.repository import GLib

ITEM = "org.kde.StatusNotifierItem"
MENU = "com.canonical.dbusmenu"
args = sys.argv[1:]
use_path = "--path-register" in args
use_pixmap = "--pixmap" in args
status = "Passive" if "--passive" in args else "Active"
icon_name = args[args.index("--name") + 1] if "--name" in args else "utilities-terminal"
log = lambda *a: print("[sni-test]", *a, flush=True)


def pixmap(size, rgb):
    r, g, b = rgb
    data = bytearray()
    for y in range(size):
        for x in range(size):
            inside = (x - size / 2) ** 2 + (y - size / 2) ** 2 < (size / 2 - 1) ** 2
            data += bytes([255 if inside else 0, r, g, b])  # ARGB32 big-endian
    return dbus.Struct((dbus.Int32(size), dbus.Int32(size), dbus.ByteArray(bytes(data))), signature="iiay")


class Item(dbus.service.Object):
    def __init__(self, bus, path):
        super().__init__(bus, path)
        self.checked = True

    def props(self):
        return {
            "Category": "ApplicationStatus",
            "Id": "sni-test",
            "Title": "Тестовый значок",
            "Status": status,
            "IconName": "" if use_pixmap else icon_name,
            "IconPixmap": dbus.Array([pixmap(22, (0x3d, 0x8b, 0xfd)), pixmap(44, (0x3d, 0x8b, 0xfd))], signature="(iiay)")
            if use_pixmap
            else dbus.Array([], signature="(iiay)"),
            "IconThemePath": "",
            "AttentionIconName": "",
            "OverlayIconName": "",
            "ToolTip": dbus.Struct(("", dbus.Array([], signature="(iiay)"), "Тест", "Подсказка значка"), signature="sa(iiay)ss"),
            "ItemIsMenu": dbus.Boolean(False),
            "Menu": dbus.ObjectPath("/MenuBar"),
        }

    @dbus.service.method(dbus.PROPERTIES_IFACE, in_signature="ss", out_signature="v")
    def Get(self, iface, name):
        return self.props()[name]

    @dbus.service.method(dbus.PROPERTIES_IFACE, in_signature="s", out_signature="a{sv}")
    def GetAll(self, iface):
        return self.props()

    @dbus.service.method(ITEM, in_signature="ii")
    def Activate(self, x, y):
        log("Activate", x, y)

    @dbus.service.method(ITEM, in_signature="ii")
    def SecondaryActivate(self, x, y):
        log("SecondaryActivate", x, y)

    @dbus.service.method(ITEM, in_signature="ii")
    def ContextMenu(self, x, y):
        log("ContextMenu", x, y)

    @dbus.service.method(ITEM, in_signature="is")
    def Scroll(self, delta, orient):
        log("Scroll", delta, orient)

    @dbus.service.signal(ITEM)
    def NewIcon(self):
        pass


def entry(id_, props, children=()):
    return dbus.Struct(
        (dbus.Int32(id_), dbus.Dictionary(props, signature="sv"), dbus.Array(list(children), signature="v")),
        signature="ia{sv}av",
    )


class Menu(dbus.service.Object):
    def __init__(self, bus, item):
        super().__init__(bus, "/MenuBar")
        self.item = item

    def layout(self):
        sub = [
            entry(21, {"label": "Подпункт А"}),
            entry(22, {"label": "Подпункт Б", "enabled": dbus.Boolean(False)}),
        ]
        return entry(0, {"children-display": "submenu"}, [
            entry(1, {"label": "_Открыть окно", "icon-name": "window-new"}),
            entry(2, {"label": "Показывать уведомления", "toggle-type": "checkmark",
                      "toggle-state": dbus.Int32(1 if self.item.checked else 0)}),
            entry(3, {"type": "separator"}),
            entry(4, {"label": "Ещё", "children-display": "submenu"}, sub),
            entry(5, {"label": "Скрытый", "visible": dbus.Boolean(False)}),
            entry(6, {"label": "Выход", "icon-name": "application-exit"}),
        ])

    @dbus.service.method(MENU, in_signature="iias", out_signature="u(ia{sv}av)")
    def GetLayout(self, parent, depth, names):
        return dbus.UInt32(1), self.layout()

    @dbus.service.method(MENU, in_signature="i", out_signature="b")
    def AboutToShow(self, id_):
        return False

    @dbus.service.method(MENU, in_signature="isvu")
    def Event(self, id_, event, data, ts):
        log("Event", id_, event)
        if id_ == 2:
            self.item.checked = not self.item.checked
            self.LayoutUpdated(dbus.UInt32(2), dbus.Int32(0))
        if id_ == 6:
            loop.quit()

    @dbus.service.method(MENU, in_signature="aias", out_signature="a(ia{sv})")
    def GetGroupProperties(self, ids, names):
        return dbus.Array([], signature="(ia{sv})")

    @dbus.service.signal(MENU, signature="ui")
    def LayoutUpdated(self, rev, parent):
        pass


DBusGMainLoop(set_as_default=True)
bus = dbus.SessionBus()
item = Item(bus, "/StatusNotifierItem")
menu = Menu(bus, item)
watcher = bus.get_object("org.kde.StatusNotifierWatcher", "/StatusNotifierWatcher")
if use_path:
    watcher.RegisterStatusNotifierItem("/StatusNotifierItem", dbus_interface="org.kde.StatusNotifierWatcher")
else:
    name = "org.kde.StatusNotifierItem-sni-test-%d" % __import__("os").getpid()
    bus_name = dbus.service.BusName(name, bus)
    watcher.RegisterStatusNotifierItem(name, dbus_interface="org.kde.StatusNotifierWatcher")
log("зарегистрирован", "путём" if use_path else "именем")
loop = GLib.MainLoop()
loop.run()
