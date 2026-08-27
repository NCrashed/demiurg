"""Check an exported animation against Blender, bone by bone, frame by frame.

`headless_export.py --expected <file>` writes where Blender puts each bone at
each frame, in voxel space. This runs `demiurg --dump-pose` at the same times
and diffs the two. A screenshot only says an animation looks wrong; this says
which bone is off and by how many voxels — which is the difference between
finding a mirrored quaternion in a minute and staring at renders for an hour.

    python3 blender/tests/compare_poses.py \
        --expected /tmp/expected.json --rig /tmp/hero.demiurg \
        --demiurg ./target/debug/demiurg --clip wave

Positions are compared relative to the root bone on both sides, so a global
offset can't hide a real error. Exits non-zero on a mismatch.
"""

import argparse
import json
import subprocess
import sys

# A tenth of a voxel: far tighter than anything visible, loose enough for the
# f32 the solver works in and the five decimals the manifest rounds to.
TOLERANCE = 0.1


def dump_pose(demiurg, rig_path, clip, time_ms):
    """`{bone: (x, y, z)}` from `demiurg --dump-pose`, relative to the root."""
    result = subprocess.run(
        [demiurg, rig_path, "--dump-pose", "--clip", clip, "--time", str(time_ms)],
        capture_output=True,
        text=True,
        check=True,
    )
    bones = {}
    for line in result.stdout.splitlines():
        if line.startswith("#") or not line.strip():
            continue
        fields = line.split("\t")
        bones[fields[0]] = tuple(float(v) for v in fields[1:4])
    if not bones:
        raise RuntimeError(f"no bones in --dump-pose output:\n{result.stdout}\n{result.stderr}")
    # The first line is the root, matching how the expectations are written.
    origin = next(iter(bones.values()))
    return {name: tuple(p[i] - origin[i] for i in range(3)) for name, p in bones.items()}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--expected", required=True)
    parser.add_argument("--rig", required=True)
    parser.add_argument("--demiurg", default="demiurg")
    parser.add_argument("--clip")
    args = parser.parse_args()

    with open(args.expected, encoding="utf-8") as f:
        expected = json.load(f)
    clip = args.clip or expected["clip"]

    worst = 0.0
    failures = []
    for frame in expected["frames"]:
        actual = dump_pose(args.demiurg, args.rig, clip, frame["t_ms"])
        for name, want in frame["bones"].items():
            got = actual.get(name)
            if got is None:
                failures.append(f"t={frame['t_ms']}ms {name}: missing from the exported rig")
                continue
            error = max(abs(got[i] - want[i]) for i in range(3))
            worst = max(worst, error)
            if error > TOLERANCE:
                failures.append(
                    f"t={frame['t_ms']}ms {name}: blender "
                    f"({want[0]:.3f}, {want[1]:.3f}, {want[2]:.3f}) vs demiurg "
                    f"({got[0]:.3f}, {got[1]:.3f}, {got[2]:.3f}) — off by {error:.3f} voxels"
                )

    frames = len(expected["frames"])
    bones = len(expected["frames"][0]["bones"]) if frames else 0
    if failures:
        for line in failures[:20]:
            print(f"  {line}")
        if len(failures) > 20:
            print(f"  ... and {len(failures) - 20} more")
        print(f"RESULT: FAIL {len(failures)} mismatch(es) over {frames} frames x {bones} bones")
        return 1
    print(
        f"RESULT: OK {frames} frames x {bones} bones agree with Blender "
        f"(worst {worst:.4f} voxels)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
