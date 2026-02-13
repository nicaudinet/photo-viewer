import pytest
from PIL import Image

from lib.photo_viewer import PhotoViewer
from lib.state import ImageState, IMAGE_EXTENSIONS
from lib.pointed_list import PointedList


@pytest.fixture
def tmp_image_dir(tmp_path):
    """
    Create a temporary directory for test images.
    """
    image_dir = tmp_path / "images"
    image_dir.mkdir()
    return image_dir


@pytest.fixture
def tmp_images(tmp_image_dir):
    """
    Create real image files that PIL can process.

    Creates a 100x50 pixel image for each supported extension so we can
    detect rotation by checking if dimensions become 50x100.
    """
    images = []
    names = [f"{i}.{ext}" for i, ext in enumerate(IMAGE_EXTENSIONS)]
    for name in names:
        img_path = tmp_image_dir / name
        img = Image.new("RGB", (100, 50), color="red")
        img.save(img_path)
        images.append(img_path)
    return sorted(images)


@pytest.fixture
def image_state(tmp_image_dir, tmp_images):
    """
    Create an ImageState with temp images for testing.
    """
    cache_dir = tmp_image_dir / ".photo-viewer"
    return ImageState(
        image_paths=PointedList(tmp_images),
        favourites=set(),
        to_delete=set(),
        image_dir=tmp_image_dir,
        cache_dir=cache_dir,
        favourites_file=cache_dir / "favourites",
        to_delete_file=cache_dir / "to_delete",
    )


@pytest.fixture
def single_view(qtbot, image_state):
    """
    Create a PhotoViewer with a SingleView loaded from image_state.
    """
    viewer = PhotoViewer(None)
    qtbot.addWidget(viewer)
    viewer.swap_to_single_view(image_state)
    viewer.show()
    qtbot.waitExposed(viewer)
    qtbot.waitActive(viewer)
    return viewer


@pytest.fixture
def wall_view(qtbot, image_state):
    """
    Create a PhotoViewer with a WallView loaded from image_state.
    """
    viewer = PhotoViewer(None)
    qtbot.addWidget(viewer)
    viewer.swap_to_wall_view(image_state)
    viewer.show()
    qtbot.waitExposed(viewer)
    qtbot.waitActive(viewer)
    return viewer
