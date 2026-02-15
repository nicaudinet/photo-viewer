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


class TestWallViewThumbnailClick:

    def test_clicking_thumbnail_swaps_to_single_view(
        self,
        qtbot,
        wall_view,
        image_state,
    ):
        masonry_wall = wall_view.centralWidget().masonry_wall
        thumb = masonry_wall.thumbnails[1]
        qtbot.mouseClick(thumb, Qt.MouseButton.LeftButton)
        assert isinstance(wall_view.centralWidget(), SingleView)
        assert image_state.image_paths.index == 1


class TestWallViewBuildWallIcons:

    def test_build_wall_shows_favourite_icon(
        self,
        qtbot,
        wall_view,
        image_state,
    ):
        image_state.favourite(image_state.image_paths.list[0])
        masonry_wall = wall_view.centralWidget().masonry_wall
        masonry_wall.thumbnails[0].is_favourite = True
        masonry_wall.build_wall()
        assert masonry_wall.thumbnails[0].star_label.isVisible()

    def test_build_wall_shows_delete_icon(
        self,
        qtbot,
        wall_view,
        image_state,
    ):
        image_state.delete(image_state.image_paths.list[0])
        masonry_wall = wall_view.centralWidget().masonry_wall
        masonry_wall.thumbnails[0].to_delete = True
        masonry_wall.build_wall()
        assert masonry_wall.thumbnails[0].delete_label.isVisible()

    def test_filter_favourites_hides_star_icon(
        self,
        qtbot,
        wall_view,
        image_state,
    ):
        image_state.favourite(image_state.image_paths.list[0])
        masonry_wall = wall_view.centralWidget().masonry_wall
        masonry_wall.thumbnails[0].is_favourite = True
        masonry_wall.show_only_favourites = True
        masonry_wall.build_wall()
        # When filtering to only favourites, star icon is hidden
        assert not masonry_wall.thumbnails[0].star_label.isVisible()

    def test_filter_to_delete_hides_delete_icon(
        self,
        qtbot,
        wall_view,
        image_state,
    ):
        image_state.delete(image_state.image_paths.list[0])
        masonry_wall = wall_view.centralWidget().masonry_wall
        masonry_wall.thumbnails[0].to_delete = True
        masonry_wall.show_only_to_delete = True
        masonry_wall.build_wall()
        # When filtering to only to-delete, delete icon is hidden
        assert not masonry_wall.thumbnails[0].delete_label.isVisible()
