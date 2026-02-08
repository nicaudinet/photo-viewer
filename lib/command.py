from typing import Callable, List, NamedTuple

from PySide6.QtCore import Qt
from PySide6.QtGui import QKeyEvent


class Command(NamedTuple):
    key: Qt.Key
    modifiers: Qt.KeyboardModifier
    description: str
    action: Callable[[], None]


NoModifier = Qt.KeyboardModifier(0)


MODIFIER_BITMASK = (
    Qt.KeyboardModifier.ControlModifier | Qt.KeyboardModifier.ShiftModifier
)


def handle_key_event(event: QKeyEvent, commands: List[Command]) -> bool:
    event_mods = event.modifiers() & MODIFIER_BITMASK
    for cmd in commands:
        if event.key() == cmd.key and event_mods == cmd.modifiers:
            cmd.action()
            event.accept()
            return True
    return False
