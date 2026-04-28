import os
import sys
from pathlib import Path
from PySide6.QtWidgets import QApplication
from lib.photo_viewer import PhotoViewer


def main() -> None:
    # On Linux, use native Wayland backend for correct HiDPI scaling
    if sys.platform == "linux":
        os.environ.setdefault("QT_QPA_PLATFORM", "wayland")
        os.environ.setdefault("QT_SCALE_FACTOR", "2")

    # Check for command-line argument
    filepath = None
    if len(sys.argv) > 1:
        filepath = Path(sys.argv[1])

    # Create Qt application
    app = QApplication(sys.argv)

    # Create viewer
    viewer = PhotoViewer(filepath)
    viewer.show()

    # Run Qt event loop
    sys.exit(app.exec())


if __name__ == "__main__":
    main()
