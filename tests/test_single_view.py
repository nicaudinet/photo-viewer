from PIL import Image
from PySide6.QtCore import Qt


class TestSingleViewNavigation:

    def test_right_arrow_advances_to_next_image(
        self,
        qtbot,
        single_view,
        image_state,
    ):
        images = image_state.image_paths.list
        assert image_state.current() == images[0]

        qtbot.keyClick(single_view, Qt.Key.Key_Right)

        assert image_state.current() == images[1]

    def test_left_arrow_goes_to_previous_image(
        self,
        qtbot,
        single_view,
        image_state,
    ):
        images = image_state.image_paths.list

        qtbot.keyClick(single_view, Qt.Key.Key_Right)
        assert image_state.current() == images[1]

        qtbot.keyClick(single_view, Qt.Key.Key_Left)

        assert image_state.current() == images[0]

    def test_right_arrow_wraps_to_first_image(
        self,
        qtbot,
        single_view,
        image_state,
    ):
        images = image_state.image_paths.list

        for _ in range(len(images) - 1):
            qtbot.keyClick(single_view, Qt.Key.Key_Right)
        assert image_state.current() == images[-1]

        qtbot.keyClick(single_view, Qt.Key.Key_Right)

        assert image_state.current() == images[0]

    def test_left_arrow_wraps_to_last_image(
        self,
        qtbot,
        single_view,
        image_state,
    ):
        images = image_state.image_paths.list
        assert image_state.current() == images[0]

        qtbot.keyClick(single_view, Qt.Key.Key_Left)

        assert image_state.current() == images[-1]


class TestSingleViewRotation:

    def test_r_rotates_current_image(
        self,
        qtbot,
        single_view,
        image_state,
    ):
        image_path = image_state.current()
        assert Image.open(image_path).size == (100, 50)

        qtbot.keyClick(single_view, Qt.Key.Key_R)

        assert Image.open(image_path).size == (50, 100)

    def test_r_only_rotates_current_image(
        self,
        qtbot,
        single_view,
        image_state,
    ):
        images = image_state.image_paths.list

        qtbot.keyClick(single_view, Qt.Key.Key_R)

        for image_path in images[1:]:
            assert Image.open(image_path).size == (100, 50)
