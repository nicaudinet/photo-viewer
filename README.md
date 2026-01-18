To build in a toolbox on Fedora Silverblue:

```bash
toolbox create photo-viewer
toolbox enter photo-viewer
sudo dnf install -y \
    python3.11 \
    python3.11-devel \
    gcc \
    patchelf \
    mesa-libGL \
    mesa-libEGL \
    libatomic \
    freetype \
    libxkbcommon \
    libxkbcommon-x11 \
    dbus-libs \
    fontconfig \
    xcb-util-cursor \
    xcb-util-wm \
    xcb-util-keysyms \
    gdk-pixbuf2 \
    atk \
    gtk3
python3.11 -m ensurepip --upgrade
./build.sh
```

Then, I:

1) Moved the binary to `~/.local/bin` to put it on my path

2) Added the following ~/.local/share/applications/PhotoViewer.desktop file to make it available via wofi/dmenu:

```
[Desktop Entry]
Name=PhotoViewer
Comment=Open images with PhotoViewer
Exec=/home/nicaudinet/.local/bin/main.linux %F
Icon=/home/nicaudinet/.local/share/icons/photo-viewer.png
Terminal=false
Type=Application
Categories=Graphics;Viewer;
StartupNotify=true
MimeType=image/png;image/jpeg;image/jpg;image/gif;image/bmp;
```

3) Added the camera.png icon to home/nicaudinet/.local/share/icons/photo-viewer.png

4) Refreshed the XDG mimetype database to make PhotoViewer the default app with which to open PNG or JPEG files

```
update-desktop-database ~/.local/share/applications/
```
