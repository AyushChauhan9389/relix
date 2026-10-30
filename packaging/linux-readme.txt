Relix (portable, Linux)
=======================

Needs libmpv from your distribution:
  Debian/Ubuntu: sudo apt install libmpv2
  Fedora:        sudo dnf install mpv-libs
  Arch:          sudo pacman -S mpv

Run ./relix-app. Nothing is installed: your addons, My List, progress and
account sign-in are kept in the "data" folder next to it, and Relix updates
itself in place.

Delete the file named "portable" to keep data in ~/.local/share/relix instead.

https://relix.carbonkit.tech
