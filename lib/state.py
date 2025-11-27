from typing import Set
from dataclasses import dataclass
from pathlib import Path

from lib.pointed_list import PointedList


@dataclass(frozen=True)
class ImageState:

    image_paths: PointedList[Path]
    favourites: Set[Path]
    to_delete: Set[Path]

    def current(self) -> Path:
        return self.image_paths.current()

    def prev(self) -> None:
        self.image_paths.prev()

    def next(self) -> None:
        self.image_paths.next()

    def goto(self, index: int) -> None:
        self.image_paths.goto(index)

    def like(self, image_path: Path) -> None:
        assert image_path in self.image_paths
        self.favourites.add(image_path)

    def dislike(self, image_path: Path) -> None:
        self.favourites.remove(image_path)

    def delete(self, image_path: Path) -> None:
        assert image_path in self.image_paths
        self.to_delete.add(image_path)

    def restore(self, image_path: Path) -> None:
        self.to_delete.remove(image_path)
