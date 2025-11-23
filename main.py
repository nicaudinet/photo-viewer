import sys
from PySide6.QtWidgets import QApplication
from lib.photo_viewer import PhotoViewer


if __name__ == "__main__":
    app = QApplication(sys.argv)
    viewer = PhotoViewer()
    viewer.show()
    sys.exit(app.exec())
