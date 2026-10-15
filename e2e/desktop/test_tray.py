"""End-to-end test of the tray icon: with a fake StatusNotifier host that
comes up after the app, the app adds its icon, its menu has the expected
items, and its Quit item quits the app.

Run via ./run.sh after test_desktop.py (it reuses that test's account), on
the same session bus and display.
"""

import asyncio
import subprocess
import time
from pathlib import Path

from dbus_next import Message, MessageType, Variant
from dbus_next.aio import MessageBus
from dbus_next.constants import PropertyAccess
from dbus_next.service import ServiceInterface, dbus_property, method, signal

# The app, through app.sh, which keeps its output.
APP = str(Path(__file__).with_name("app.sh"))
WATCHER = "org.kde.StatusNotifierWatcher"


class Watcher(ServiceInterface):
    """Just enough of a StatusNotifier host for libayatana-appindicator."""

    def __init__(self):
        super().__init__(WATCHER)
        self.items: list[str] = []

    @method()
    def RegisterStatusNotifierItem(self, service: "s"):  # noqa: N802 - D-Bus names
        self.items.append(service)
        self.StatusNotifierItemRegistered(service)

    @method()
    def RegisterStatusNotifierHost(self, service: "s"):  # noqa: N802
        pass

    @dbus_property(access=PropertyAccess.READ)
    def RegisteredStatusNotifierItems(self) -> "as":  # noqa: N802
        return self.items

    @dbus_property(access=PropertyAccess.READ)
    def IsStatusNotifierHostRegistered(self) -> "b":  # noqa: N802
        return True

    @dbus_property(access=PropertyAccess.READ)
    def ProtocolVersion(self) -> "i":  # noqa: N802
        return 0

    @signal()
    def StatusNotifierItemRegistered(self, service) -> "s":  # noqa: N802
        return service


def menu_items(layout, depth=0, items=None) -> list[dict]:
    items = [] if items is None else items
    id_, props, children = layout
    items.append(
        {
            "id": id_,
            "depth": depth,
            "label": props["label"].value if "label" in props else None,
            "separator": "type" in props and props["type"].value == "separator",
            "enabled": props["enabled"].value if "enabled" in props else True,
        }
    )
    for child in children:
        menu_items(child.value, depth + 1, items)
    return items


async def call(bus, timeout=2.0, **message):
    return await asyncio.wait_for(bus.call(Message(**message)), timeout)


async def find_item(bus, path: str):
    """The bus name and properties of the StatusNotifierItem at `path`."""
    names = (
        await call(
            bus,
            destination="org.freedesktop.DBus",
            path="/org/freedesktop/DBus",
            interface="org.freedesktop.DBus",
            member="ListNames",
        )
    ).body[0]
    for name in names:
        if not name.startswith(":"):
            continue
        try:
            reply = await call(
                bus,
                timeout=1.0,
                destination=name,
                path=path,
                interface="org.freedesktop.DBus.Properties",
                member="GetAll",
                signature="s",
                body=["org.kde.StatusNotifierItem"],
            )
        except asyncio.TimeoutError:
            continue
        if reply.message_type == MessageType.METHOD_RETURN:
            return name, {k: v.value for k, v in reply.body[0].items()}
    raise AssertionError(f"no StatusNotifierItem at {path}")


async def run() -> None:
    app = subprocess.Popen([APP])
    try:
        # The host comes up after the app, which follows it.
        await asyncio.sleep(3)
        assert app.poll() is None, "the app exited without a StatusNotifier host"
        bus = await MessageBus().connect()
        watcher = Watcher()
        bus.export("/StatusNotifierWatcher", watcher)
        await bus.request_name(WATCHER)

        deadline = time.monotonic() + 20
        while not watcher.items and time.monotonic() < deadline:
            await asyncio.sleep(0.2)
        assert watcher.items, "the app didn't add its tray icon"
        await asyncio.sleep(1)

        owner, item = await find_item(bus, watcher.items[0])
        assert item["Status"] == "Active", item
        layout = await call(
            bus,
            destination=owner,
            path=item["Menu"],
            interface="com.canonical.dbusmenu",
            member="GetLayout",
            signature="iias",
            body=[0, -1, []],
        )
        items = menu_items(layout.body[1])
        labels = [i["label"] for i in items if i["label"] and not i["separator"]]
        assert labels[0].endswith(("unread message", "unread messages")), labels
        assert labels[1:] == [
            "Open Cumulas Mail",
            "New message",
            "Check for new mail",
            "Pause notifications",
            "For 1 hour",
            "Until tomorrow",
            "Resume",
            "Quit Cumulas Mail",
        ], labels
        by_label = {i["label"]: i for i in items}
        assert not by_label["Resume"]["enabled"], "Resume while not paused"

        await call(
            bus,
            destination=owner,
            path=item["Menu"],
            interface="com.canonical.dbusmenu",
            member="Event",
            signature="isvu",
            body=[by_label["Quit Cumulas Mail"]["id"], "clicked", Variant("s", ""), 0],
        )
        assert app.wait(timeout=15) == 0, "Quit didn't quit cleanly"
    finally:
        if app.poll() is None:
            app.kill()


if __name__ == "__main__":
    asyncio.run(run())
    print("tray end-to-end test passed")
