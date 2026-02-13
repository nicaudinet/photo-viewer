from PySide6.QtCore import Qt

from lib.view.single_view import SingleView

from tests.fixtures import tmp_image_dir, tmp_images, image_state, wall_view

ShiftModifier = Qt.KeyboardModifier.ShiftModifier


class TestWallViewFilter:

    def test_shift_f_toggles_show_only_favourites_on(self, qtbot, wall_view):
        masonry_wall = wall_view.centralWidget().masonry_wall
        assert not masonry_wall.show_only_favourites
        qtbot.keyClick(wall_view, Qt.Key.Key_F, ShiftModifier)
        assert masonry_wall.show_only_favourites

    def test_shift_f_toggles_show_only_favourites_off(self, qtbot, wall_view):
        masonry_wall = wall_view.centralWidget().masonry_wall
        qtbot.keyClick(wall_view, Qt.Key.Key_F, ShiftModifier)
        assert masonry_wall.show_only_favourites
        qtbot.keyClick(wall_view, Qt.Key.Key_F, ShiftModifier)
        assert not masonry_wall.show_only_favourites

    def test_shift_d_toggles_show_only_to_delete_on(self, qtbot, wall_view):
        masonry_wall = wall_view.centralWidget().masonry_wall
        assert not masonry_wall.show_only_to_delete
        qtbot.keyClick(wall_view, Qt.Key.Key_D, ShiftModifier)
        assert masonry_wall.show_only_to_delete

    def test_shift_d_toggles_show_only_to_delete_off(self, qtbot, wall_view):
        masonry_wall = wall_view.centralWidget().masonry_wall
        qtbot.keyClick(wall_view, Qt.Key.Key_D, ShiftModifier)
        assert masonry_wall.show_only_to_delete
        qtbot.keyClick(wall_view, Qt.Key.Key_D, ShiftModifier)
        assert not masonry_wall.show_only_to_delete


class TestWallViewSwap:

    def test_w_calls_swap_to_single_view(self, qtbot, wall_view):
        qtbot.keyClick(wall_view, Qt.Key.Key_W)
        assert isinstance(wall_view.centralWidget(), SingleView)
