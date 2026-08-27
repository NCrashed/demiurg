"""Finding the `demiurg-convert` binary shipped inside the addon.

A release zip carries the converter under `bin/<platform>/`, so an artist
installs one file and nothing else — no second download, no path to set. This
is the part that decides which of those to run.

Deliberately free of `bpy`: the platform mapping is the one piece of the addon
that cannot be tested on the machine it matters most for (whoever builds the
release is rarely on the same OS as whoever installs it), so it is testable
without Blender — see `blender/tests/test_pure.py`.
"""

import os
import platform
import sys

CONVERTER = "demiurg-convert"


def platform_tag(system=None, machine=None):
    """This machine's folder name under `bin/`, e.g. `linux-x86_64`.

    `system` / `machine` default to the running interpreter's; they are
    parameters so every branch can be tested from one machine.
    """
    system = sys.platform if system is None else system
    machine = platform.machine() if machine is None else machine
    name = {"win32": "windows", "cygwin": "windows", "darwin": "macos"}.get(system, "linux")
    arch = machine.lower()
    arch = {"amd64": "x86_64", "x86_64": "x86_64", "aarch64": "arm64", "arm64": "arm64"}.get(
        arch, arch
    )
    return f"{name}-{arch}"


def converter_name(system=None):
    """The binary's file name on this platform."""
    system = sys.platform if system is None else system
    return CONVERTER + (".exe" if system in ("win32", "cygwin") else "")


def bundled_converter(root=None):
    """The converter shipped with the addon, or `None` if this build has none.

    A zip loses the executable bit on some install routes, so it is restored
    here rather than trusted.
    """
    root = os.path.dirname(os.path.abspath(__file__)) if root is None else root
    path = os.path.join(root, "bin", platform_tag(), converter_name())
    if not os.path.isfile(path):
        return None
    if not os.access(path, os.X_OK):
        try:
            os.chmod(path, os.stat(path).st_mode | 0o111)
        except OSError:
            return None
    return path
