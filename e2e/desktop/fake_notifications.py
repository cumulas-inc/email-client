"""A fake notification server (org.freedesktop.Notifications) for the desktop
test: it records what the app shows and withdraws, and clicks notifications
and their buttons as a user would. It runs on the test's session bus in a
thread of its own.
"""

import asyncio
import threading
import time

from dbus_next import Variant
from dbus_next.aio import MessageBus
from dbus_next.service import ServiceInterface, method, signal

PATH = "/org/freedesktop/Notifications"
NAME = "org.freedesktop.Notifications"


class _Notifications(ServiceInterface):
    def __init__(self, server: "FakeNotificationServer"):
        super().__init__(NAME)
        self._server = server

    @method()
    def Notify(  # noqa: N802 - D-Bus names
        self,
        app_name: "s",
        replaces_id: "u",
        app_icon: "s",
        summary: "s",
        body: "s",
        actions: "as",
        hints: "a{sv}",
        expire_timeout: "i",
    ) -> "u":
        return self._server._notify(
            {
                "app_name": app_name,
                "replaces_id": replaces_id,
                "app_icon": app_icon,
                "summary": summary,
                "body": body,
                "actions": list(actions),
                "hints": {k: v.value if isinstance(v, Variant) else v for k, v in hints.items()},
                "expire_timeout": expire_timeout,
            }
        )

    @method()
    def CloseNotification(self, id: "u"):  # noqa: N802
        self._server._close(id)

    @method()
    def GetCapabilities(self) -> "as":  # noqa: N802
        return ["actions", "body", "body-markup", "persistence"]

    @method()
    def GetServerInformation(self) -> "ssss":  # noqa: N802
        return ["fake", "cumulas-e2e", "1.0", "1.2"]

    @signal()
    def ActionInvoked(self, id: "u", action_key: "s") -> "us":  # noqa: N802
        return [id, action_key]

    @signal()
    def ActivationToken(self, id: "u", activation_token: "s") -> "us":  # noqa: N802
        return [id, activation_token]

    @signal()
    def NotificationClosed(self, id: "u", reason: "u") -> "uu":  # noqa: N802
        return [id, reason]


class FakeNotificationServer:
    def __init__(self):
        self._lock = threading.Lock()
        self._next_id = 100
        self.notified: list[dict] = []
        self.closed: list[int] = []
        self._loop: asyncio.AbstractEventLoop | None = None
        self._iface: _Notifications | None = None

    # --- the server side ------------------------------------------------------

    def _notify(self, notification: dict) -> int:
        with self._lock:
            if notification["replaces_id"]:
                notification["id"] = notification["replaces_id"]
            else:
                self._next_id += 1
                notification["id"] = self._next_id
            self.notified.append(notification)
            return notification["id"]

    def _close(self, id: int) -> None:
        with self._lock:
            self.closed.append(id)

    def start(self) -> None:
        """Owns the name on the session bus; returns once it does."""
        ready = threading.Event()
        failure: list[BaseException] = []

        async def serve():
            try:
                bus = await MessageBus().connect()
                self._iface = _Notifications(self)
                bus.export(PATH, self._iface)
                await bus.request_name(NAME)
            except BaseException as e:  # noqa: BLE001 - reported to the caller
                failure.append(e)
                ready.set()
                return
            ready.set()
            await asyncio.get_running_loop().create_future()

        def run():
            self._loop = asyncio.new_event_loop()
            self._loop.run_until_complete(serve())

        threading.Thread(target=run, daemon=True, name="fake-notifications").start()
        ready.wait(10)
        if failure:
            raise failure[0]

    # --- the test's side -------------------------------------------------------

    def click(self, notification: dict, action: str = "default", token: str | None = None) -> None:
        """Clicks a notification (`default`) or one of its buttons."""
        assert self._loop and self._iface

        def emit():
            if token:
                self._iface.ActivationToken(notification["id"], token)
            self._iface.ActionInvoked(notification["id"], action)
            self._iface.NotificationClosed(notification["id"], 2)

        self._loop.call_soon_threadsafe(emit)

    def wait_for(self, predicate, timeout: float = 15.0, what: str = "a notification") -> dict:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            with self._lock:
                found = next((n for n in self.notified if predicate(n)), None)
            if found:
                return found
            time.sleep(0.1)
        with self._lock:
            seen = [(n["summary"], n["body"]) for n in self.notified]
        raise AssertionError(f"timed out waiting for {what}; notifications so far: {seen}")

    def wait_closed(self, id: int, timeout: float = 15.0) -> None:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            with self._lock:
                if id in self.closed:
                    return
            time.sleep(0.1)
        raise AssertionError(f"notification {id} was not withdrawn; closed: {self.closed}")
