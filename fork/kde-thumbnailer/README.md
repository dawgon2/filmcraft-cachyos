# FilmCraft project thumbnails in Dolphin (fork-only)

KDE's file manager doesn't read freedesktop `.thumbnailer` files; it needs a KIO
`ThumbnailCreator` plugin, which is C++. This folder is that plugin, kept as small as possible:
it runs `filmcraft-cli thumbnail <file> <size> <out.png>` (the Rust side draws the picture: the
project's active sequence, a tenth of the way in) with a timeout and hands the PNG to Dolphin.

Fork-only (FORK-PLAN §1a.46): not part of the Cargo workspace, nothing else references this
folder, and deleting it removes the feature. Listed in `docs/fork/FORK-ONLY.md`. Original code,
MIT; it links KDE Frameworks (KIO, LGPL) at run time like any KIO plugin.

## Build and test

Needs extra-cmake-modules, KF6 KIO, Qt 6 (Arch / CachyOS: `extra-cmake-modules kio qt6-base`).

```sh
cmake -B build -DCMAKE_INSTALL_PREFIX=/usr -DCMAKE_BUILD_TYPE=Release
cmake --build build
ctest --test-dir build --output-on-failure
```

The test loads the built plugin the way Dolphin's thumbnail worker does and checks a good CLI,
a failing one, a bad PNG, no output, a missing CLI, a missing project and a folder (through the
`FILMCRAFT_CLI` variable). `build/thumbtest <plugin> <dir> <filmcraft-cli> <project.fcproj>`
also renders a real project.

## Install

**Package:** the L2 `PKGBUILD` builds it as the split package `filmcraft-kde-thumbnailer`
(installs `/usr/lib/qt6/plugins/kf6/thumbcreator/filmcraftthumbnail.so`; needs the main package
for the MIME type and `filmcraft-cli`).

**User-local (the fork's own build in `~/Apps/filmcraft-builds/latest/`), no root:**

```sh
cmake -B build -DCMAKE_INSTALL_PREFIX="$HOME/.local" \
      -DFILMCRAFT_CLI="$HOME/Apps/filmcraft-builds/latest/filmcraft-cli"
cmake --build build && cmake --install build
# the project MIME type, if FilmCraft isn't installed as a package
install -Dm644 ../../packaging/linux/ai.storyteller.filmcraft.mime.xml \
    ~/.local/share/mime/packages/ai.storyteller.filmcraft.xml
update-mime-database ~/.local/share/mime
# let KDE find plugins under ~/.local (takes effect at the next login)
mkdir -p ~/.config/plasma-workspace/env
echo 'export QT_PLUGIN_PATH="$HOME/.local/lib/qt6/plugins${QT_PLUGIN_PATH:+:$QT_PLUGIN_PATH}"' \
    > ~/.config/plasma-workspace/env/filmcraft-thumbnailer.sh
```

Then log out and in, and tick **FilmCraft projects** in Dolphin ▸ Configure Dolphin ▸
Interface ▸ Previews. `FILMCRAFT_CLI` in the environment overrides the configured path.
To remove: delete `~/.local/lib/qt6/plugins/kf6/thumbcreator/filmcraftthumbnail.so` and the env
file.
