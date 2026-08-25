"""Install the addon as a user would, then run the export operator itself.

`headless_export.py` calls the export function directly, which is the quick
loop; this one goes through installation and `bpy.ops`, so it also covers
registration, the preferences lookup, and the operator's error reporting —
everything between the menu click and the export.

    cd blender && zip -qr /tmp/demiurg_export.zip demiurg_export && cd ..
    blender --background --python blender/tests/headless_install.py -- \
        --zip /tmp/demiurg_export.zip --out /tmp/hero.demiurg \
        --converter ./target/debug/demiurg-convert

Prints `RESULT: OK` or `RESULT: FAIL` — Blender swallows a script's exit code.
"""

import os
import sys
import traceback

import bpy

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from headless_export import build_scene  # noqa: E402

MODULE = "demiurg_export"


def install(zip_path):
    """Install `zip_path` and return the key it took in `preferences.addons`.

    Blender 4.2+ installs this as an extension (it ships a
    `blender_manifest.toml`); older builds take the legacy add-on path.
    """
    try:
        bpy.ops.extensions.package_install_files(
            filepath=zip_path, repo="user_default", enable_on_install=True
        )
    except (RuntimeError, AttributeError):
        bpy.ops.preferences.addon_install(filepath=zip_path)
        bpy.ops.preferences.addon_enable(module=MODULE)
    for key in bpy.context.preferences.addons.keys():
        # An extension is keyed `bl_ext.<repo>.<module>`, a legacy add-on just
        # `<module>`.
        if key == MODULE or key.endswith(f".{MODULE}"):
            return key
    raise RuntimeError(f"installed but not registered; addons: {list(bpy.context.preferences.addons.keys())}")


def main():
    argv = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    args = dict(zip(argv[::2], argv[1::2]))
    zip_path = os.path.abspath(args["--zip"])
    out = args.get("--out", "/tmp/demiurg-blender-install-test.demiurg")
    converter = args.get("--converter")

    key = install(zip_path)
    print(f"INSTALL: registered as {key}")
    if converter:
        prefs = bpy.context.preferences.addons[key].preferences
        prefs.converter_path = os.path.abspath(converter)

    build_scene()
    result = bpy.ops.export_scene.demiurg_rig(filepath=out, voxels_per_unit=10.0, solid=True)
    if "FINISHED" not in result:
        print(f"RESULT: FAIL operator returned {result}")
        return
    if not os.path.isfile(out):
        print(f"RESULT: FAIL {out} was not written")
        return
    print(f"RESULT: OK wrote {out} ({os.path.getsize(out)} bytes)")


if __name__ == "__main__":
    try:
        main()
    except Exception:  # noqa: BLE001 — the marker line is the test's verdict
        traceback.print_exc()
        print("RESULT: FAIL")
