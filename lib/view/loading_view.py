from PySide6.QtWidgets import QWidget, QVBoxLayout, QLabel
from PySide6.QtCore import Qt, QTimer


class LoadingView(QWidget):
    """Shown while an image or directory the app was asked to open is being
    read. Distinct from EmptyView, which represents "nothing is open".

    The loading indicator is revealed only after a short delay, so a fast load
    (or an immediate fall-back to another view) shows no visual noise.
    """

    INDICATOR_DELAY_MS: int = 250

    def __init__(self, parent=None):

        super().__init__(parent)

        self.setAutoFillBackground(True)

        layout = QVBoxLayout(self)

        self.loading_label = QLabel("Loading …")
        self.loading_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.loading_label.hide()
        layout.addWidget(self.loading_label)

        # Reveal the indicator only if this view is still alive after the
        # delay. The context object (self) cancels the call if we are deleted.
        QTimer.singleShot(self.INDICATOR_DELAY_MS, self, self._reveal)

    def _reveal(self) -> None:
        self.loading_label.show()

    def commands(self):
        return []
