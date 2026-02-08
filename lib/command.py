from typing import Callable, List, NamedTuple, Optional

from PySide6.QtCore import Qt
from PySide6.QtGui import QKeyEvent


class Command(NamedTuple):
    key: Qt.Key
    modifiers: Optional[Qt.KeyboardModifier]
    description: str
    action: Callable[[], None]


NoModifier = Qt.KeyboardModifier(0)


MODIFIER_BITMASK = (
    Qt.KeyboardModifier.ControlModifier | Qt.KeyboardModifier.ShiftModifier
)


_KEY_NAMES = {
    Qt.Key.Key_Left: "\u2190",
    Qt.Key.Key_Right: "\u2192",
    Qt.Key.Key_Up: "\u2191",
    Qt.Key.Key_Down: "\u2193",
    Qt.Key.Key_Question: "?",
    Qt.Key.Key_Space: "Space",
    Qt.Key.Key_Return: "Enter",
    Qt.Key.Key_Escape: "Esc",
    Qt.Key.Key_Tab: "Tab",
    Qt.Key.Key_Backspace: "Backspace",
    Qt.Key.Key_Delete: "Delete",
}


def key_display_name(cmd: Command) -> str:
    parts = []
    if cmd.modifiers and cmd.modifiers & Qt.KeyboardModifier.ControlModifier:
        parts.append("Ctrl")
    if cmd.modifiers and cmd.modifiers & Qt.KeyboardModifier.ShiftModifier:
        parts.append("Shift")
    if cmd.key in _KEY_NAMES:
        parts.append(_KEY_NAMES[cmd.key])
    else:
        parts.append(chr(cmd.key))
    return "+".join(parts)


def handle_key_event(event: QKeyEvent, commands: List[Command]) -> bool:
    event_mods = event.modifiers() & MODIFIER_BITMASK
    for cmd in commands:
        mods_match = cmd.modifiers is None or event_mods == cmd.modifiers
        if event.key() == cmd.key and mods_match:
            cmd.action()
            event.accept()
            return True
    return False
