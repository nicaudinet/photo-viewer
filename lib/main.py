import os
import sys
from pathlib import Path
from typing import Optional

from PySide6.QtCore import QEvent
from PySide6.QtWidgets import QApplication

from lib.photo_viewer import PhotoViewer


class PhotoViewerApplication(QApplication):
    """QApplication that also handles macOS "Open With" / double-click.

    When an image is opened from Finder, macOS delivers the path as a
    QFileOpenEvent (an Apple Event) rather than on argv. The event can
    arrive before the viewer window exists, so buffer the last path until
    a viewer registers itself.
    """

    def __init__(self, argv: list[str]) -> None:
        super().__init__(argv)
        self._viewer: Optional[PhotoViewer] = None
        self._pending_path: Optional[Path] = None

    def set_viewer(self, viewer: PhotoViewer) -> None:
        self._viewer = viewer
        if self._pending_path is not None:
            viewer.load_path(self._pending_path)
            self._pending_path = None

    def event(self, event: QEvent) -> bool:
        if event.type() == QEvent.Type.FileOpen:
            path = Path(event.file())  # pyright: ignore[reportAttributeAccessIssue]
            if self._viewer is not None:
                self._viewer.load_path(path)
            else:
                self._pending_path = path
            return True
        return super().event(event)


def main() -> None:
    # On Linux, use native Wayland backend for correct HiDPI scaling
    if sys.platform == "linux":
        os.environ.setdefault("QT_QPA_PLATFORM", "wayland")
        os.environ.setdefault("QT_SCALE_FACTOR", "1")

    # Check for command-line argument
    filepath = None
    if len(sys.argv) > 1:
        filepath = Path(sys.argv[1])

    # Create Qt application
    app = PhotoViewerApplication(sys.argv)

    # Create viewer, then register it so pending Finder open-events resolve
    viewer = PhotoViewer(filepath)
    app.set_viewer(viewer)
    viewer.show()

    # Run Qt event loop
    sys.exit(app.exec())


if __name__ == "__main__":
    main()
