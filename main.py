import sys
from PySide6.QtWidgets import QApplication
from lib.photo_viewer import PhotoViewer
from pathlib import Path


if __name__ == "__main__":
    app = QApplication(sys.argv)

    filepath = None
    if len(sys.argv) > 1:
        filepath = Path(sys.argv[1])
    viewer = PhotoViewer(filepath)
    viewer.show()

    sys.exit(app.exec())
