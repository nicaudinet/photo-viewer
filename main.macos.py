import sys
from AppKit import NSApplication
from Foundation import NSObject
from PySide6.QtWidgets import QApplication
from lib.photo_viewer import PhotoViewer
from pathlib import Path


class MyAppDelegate(NSObject):

    def init(self):
        super().__init__()
        # self = super(MyAppDelegate, self).init()
        self.viewer = None
        return self

    def set_viewer(self, viewer):
        self.viewer = viewer

    def application_openFile_(self, app, filename):
        # Called when opened from Finder
        if self.viewer:
            self.viewer.load_path(Path(filename))
        return True


if __name__ == "__main__":

    # Check for command-line argument
    filepath = None
    if len(sys.argv) > 1:
        filepath = Path(sys.argv[1])

    # Create Qt application
    app = QApplication(sys.argv)

    # Create viewer
    viewer = PhotoViewer(filepath)
    viewer.show()

    # Set up macOS delegate for Finder events
    ns_app = NSApplication.sharedApplication()
    delegate = MyAppDelegate.alloc().init()
    delegate.set_viewer(viewer)
    ns_app.setDelegate_(delegate)

    # Run Qt event loop
    sys.exit(app.exec())
