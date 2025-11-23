class PointedList:
    """
    A circular list with a highlighted item and next/prev functions
    """

    def __init__(self, list):
        assert len(list) > 0, "The list in a PointedList cannot be empty"
        self.list = list
        self.index = 0

    def current(self):
        return self.list[self.index]

    def next(self):
        self.index = (self.index + 1) % len(self.list)
        return self.current()

    def prev(self):
        self.index = (self.index - 1) % len(self.list)
        return self.current()

    def goto(self, index: int):
        assert 0 <= index < len(self.list), "Index is not in the list"
        self.index = index
