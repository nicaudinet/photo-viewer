from typing import TypeVar, Generic, List, Iterator

T = TypeVar("T")


class PointedList(Generic[T]):
    """
    A circular list with a highlighted item and next/prev functions
    """

    def __init__(self, list: List[T]):
        assert len(list) > 0, "The list in a PointedList cannot be empty"
        self.list = list
        self.index = 0

    def __iter__(self) -> Iterator[T]:
        return iter(self.list)

    def current(self) -> T:
        return self.list[self.index]

    def next(self) -> T:
        self.index = (self.index + 1) % len(self.list)
        return self.current()

    def prev(self) -> T:
        self.index = (self.index - 1) % len(self.list)
        return self.current()

    def goto(self, index: int) -> None:
        assert 0 <= index < len(self.list), "Index is not in the list"
        self.index = index

    def delete(self) -> T:
        self.list.pop(self.index)
        self.index = self.index % len(self.list)
        return self.current()
