from dataclasses import dataclass
from typing import Set, Optional
from pathlib import Path
import shutil

from lib.pointed_list import PointedList


@dataclass(frozen=True)
class ImageState:

    image_paths: PointedList[Path]
    favourites: Set[Path]
    to_delete: Set[Path]

    image_dir: Path
    cache_dir: Path
    favourites_file: Path
    to_delete_file: Path

    def current(self) -> Path:
        return self.image_paths.current()

    def prev(self) -> None:
        self.image_paths.prev()

    def next(self) -> None:
        self.image_paths.next()

    def goto(self, index: int) -> None:
        self.image_paths.goto(index)

    def favourite(self, image_path: Path) -> None:
        assert image_path in self.image_paths
        self.favourites.add(image_path)
        self.save()

    def unfavourite(self, image_path: Path) -> None:
        if image_path in self.favourites:
            self.favourites.remove(image_path)
            self.save()

    def delete(self, image_path: Path) -> None:
        assert image_path in self.image_paths
        self.to_delete.add(image_path)
        self.save()

    def undelete(self, image_path: Path) -> None:
        if image_path in self.to_delete:
            self.to_delete.remove(image_path)
            self.save()

    def delete_all(self):
        while self.to_delete:
            image_path = self.current()
            if image_path in self.to_delete:
                image_path.unlink(missing_ok=True)
                self.image_paths.delete()
                self.to_delete.remove(image_path)
            else:
                self.next()
        self.save()

    def save(self) -> None:
        self.cache_dir.mkdir(exist_ok=True, parents=True)
        with self.favourites_file.open("w") as file:
            lines = [str(f) for f in self.favourites]
            file.write("\n".join(lines))
        with self.to_delete_file.open("w") as file:
            lines = [str(d) for d in self.to_delete]
            file.writelines("\n".join(lines))

    def save_favourites(self, new_dir: Path) -> None:
        for favourite in self.favourites:
            destination = new_dir / favourite.name
            if destination.exists():
                print(f"Destination {destination} already exists, skipping.")
            else:
                shutil.copy2(favourite, destination)


def load_image_state(image_dir: Path) -> Optional[ImageState]:

    if not image_dir.exists():
        raise ValueError(f"Directory {image_dir} does not exist")

    if not image_dir.is_dir():
        raise ValueError(f"{image_dir} is not a directory")

    cache_dir = image_dir / ".photo-viewer"
    favourites_file = cache_dir / "favourites"
    to_delete_file = cache_dir / "to_delete"

    exts = (".png", ".jpg", ".jpeg")
    all_files = Path(image_dir).iterdir()
    image_paths = [f for f in all_files if f.suffix.lower() in exts]
    if len(image_paths) == 0:
        return
    image_paths = sorted(image_paths)

    favourites = set()
    if not favourites_file.exists():
        print("Favourites file not found in cache. Init with empty set")
    else:
        with favourites_file.open("r") as file:
            lines = file.readlines()
        for line in lines:
            favourite_path = Path(line.strip())
            if favourite_path in image_paths:
                favourites.add(favourite_path)
            else:
                print(f"Favourite {favourite_path} not in image paths")

    to_delete = set()
    if not to_delete_file.exists():
        print("To delete file not found in cache. Init with empty set")
    else:
        with to_delete_file.open("r") as file:
            lines = file.readlines()
        for line in lines:
            to_delete_path = Path(line.strip())
            if to_delete_path in image_paths:
                to_delete.add(to_delete_path)
            else:
                print(f"To delete {to_delete_path} not in image paths")

    return ImageState(
        image_paths=PointedList(image_paths),
        favourites=favourites,
        to_delete=to_delete,
        image_dir=image_dir,
        cache_dir=cache_dir,
        favourites_file=favourites_file,
        to_delete_file=to_delete_file,
    )
