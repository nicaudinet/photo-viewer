from PySide6.QtCore import Qt

from lib.command import Command, NoModifier, key_display_name


class TestKeyDisplayName:

    def test_plain_letter_key(self):
        cmd = Command(
            key=Qt.Key.Key_Q,
            modifiers=NoModifier,
            description="Quit",
            action=lambda: None,
        )
        assert key_display_name(cmd) == "Q"

    def test_ctrl_modifier(self):
        cmd = Command(
            key=Qt.Key.Key_F,
            modifiers=Qt.KeyboardModifier.ControlModifier,
            description="Save favourites",
            action=lambda: None,
        )
        assert key_display_name(cmd) == "Ctrl+F"

    def test_shift_modifier(self):
        cmd = Command(
            key=Qt.Key.Key_F,
            modifiers=Qt.KeyboardModifier.ShiftModifier,
            description="Show only favourites",
            action=lambda: None,
        )
        assert key_display_name(cmd) == "Shift+F"

    def test_ctrl_shift_modifier(self):
        cmd = Command(
            key=Qt.Key.Key_D,
            modifiers=(
                Qt.KeyboardModifier.ControlModifier
                | Qt.KeyboardModifier.ShiftModifier
            ),
            description="Test",
            action=lambda: None,
        )
        assert key_display_name(cmd) == "Ctrl+Shift+D"

    def test_special_key_name(self):
        cmd = Command(
            key=Qt.Key.Key_Left,
            modifiers=NoModifier,
            description="Previous",
            action=lambda: None,
        )
        assert key_display_name(cmd) == "\u2190"

    def test_none_modifiers(self):
        cmd = Command(
            key=Qt.Key.Key_Q,
            modifiers=None,
            description="Quit",
            action=lambda: None,
        )
        assert key_display_name(cmd) == "Q"
