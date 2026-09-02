#!/usr/bin/env bash
# Package the Blender addon as a single zip an artist can install.
#
#   scripts/package-blender-addon.sh              # this machine's binary only
#   scripts/package-blender-addon.sh --cross windows        # + a Windows build
#   scripts/package-blender-addon.sh --with-bin windows-x86_64=/path/demiurg-convert.exe
#
# The addon shells out to `demiurg-convert`, so a zip without it would make
# every artist find, download, and point at a binary before their first export.
# This builds the host binary in release mode and lays it inside the addon
# under `bin/<platform>/`, where `operator.bundled_converter()` looks; extra
# platforms are folded in with `--with-bin` (a Windows build from CI, say).
#
# The result is one file for File > Install from Disk, and nothing else to set.
set -euo pipefail
cd "$(dirname "$0")/.."

DIST="dist"
STAGE="$DIST/stage"
ADDON="blender/demiurg_export"

extra_bins=()
cross_windows=0
host_build=1
version_override=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --version)
            [[ $# -ge 2 ]] || { echo "--version needs X.Y.Z" >&2; exit 1; }
            # A leading `v` so a git tag can be passed through unedited.
            version_override="${2#v}"
            shift 2
            ;;
        --no-host-build)
            # CI builds each platform on its own runner and assembles here;
            # there is nothing to compile at packaging time.
            host_build=0
            shift
            ;;
        --cross)
            [[ "${2:-}" == "windows" ]] || { echo "--cross only knows 'windows'" >&2; exit 1; }
            cross_windows=1
            shift 2
            ;;
        --with-bin)
            [[ $# -ge 2 ]] || { echo "--with-bin needs <platform>=<path>" >&2; exit 1; }
            extra_bins+=("$2")
            shift 2
            ;;
        -h|--help)
            sed -n '2,12p' "$0" | sed 's/^# \?//'
            exit 0
            ;;
        *)
            echo "unknown argument: $1" >&2
            exit 1
            ;;
    esac
done

# The version in the manifest is what a local build carries; a release passes
# its tag instead, so the installed extension reports the release it came from
# rather than a number that drifts on its own.
version=$(grep -m1 '^version = ' "$ADDON/blender_manifest.toml" | sed -E 's/version = "(.*)".*/\1/')
[[ -n "$version" ]] || { echo "cannot read version from $ADDON/blender_manifest.toml" >&2; exit 1; }
if [[ -n "$version_override" ]]; then
    # Blender parses this strictly; a tag that isn't `X.Y.Z` would install as a
    # broken extension, so refuse it here where the message is readable.
    [[ "$version_override" =~ ^[0-9]+\.[0-9]+\.[0-9]+([.-].*)?$ ]] \
        || { echo "--version $version_override is not X.Y.Z" >&2; exit 1; }
    version="$version_override"
fi

# Host platform, in the same spelling `platform_tag()` builds at runtime.
case "$(uname -s)" in
    Linux)  host_os=linux ;;
    Darwin) host_os=macos ;;
    MINGW*|MSYS*|CYGWIN*) host_os=windows ;;
    *) echo "unsupported host: $(uname -s)" >&2; exit 1 ;;
esac
case "$(uname -m)" in
    x86_64|amd64) host_arch=x86_64 ;;
    arm64|aarch64) host_arch=arm64 ;;
    *) host_arch=$(uname -m) ;;
esac
host_tag="$host_os-$host_arch"
exe=""
[[ "$host_os" == windows ]] && exe=".exe"

rm -rf "$STAGE"
mkdir -p "$STAGE/demiurg_export"
# Copy the addon itself, minus anything Python or an editor left behind.
tar -C "$ADDON" --exclude='__pycache__' --exclude='*.pyc' --exclude='bin' -cf - . \
    | tar -C "$STAGE/demiurg_export" -xf -

# Ride the docs along, so an artist who unzips it has the instructions in hand.
cp blender/README.md "$STAGE/demiurg_export/README.md"

# Stamp the version into the staged manifest only — the file in git keeps the
# development version, so a release build never leaves the tree dirty.
if [[ -n "$version_override" ]]; then
    # `^version` and not `version`, so `schema_version` above it is untouched.
    sed -i -E "0,/^version = \".*\"/s//version = \"$version\"/" \
        "$STAGE/demiurg_export/blender_manifest.toml"
    grep -q "^version = \"$version\"$" "$STAGE/demiurg_export/blender_manifest.toml" \
        || { echo "failed to stamp version $version into the manifest" >&2; exit 1; }
fi

bundled_tags=()
if [[ "$host_build" == 1 ]]; then
    echo "building demiurg-convert (release) for $host_tag"
    # Linux builds static against musl: a glibc binary carries an ELF
    # interpreter path NixOS does not have, and a glibc floor an older distro
    # would trip over — both of which travel with the zip.
    host_target=""
    [[ "$host_os" == linux ]] && host_target="x86_64-unknown-linux-musl"
    if [[ -n "$host_target" ]]; then
        cargo build --release -p demiurg-convert --target "$host_target"
        built="target/$host_target/release/demiurg-convert$exe"
    else
        cargo build --release -p demiurg-convert
        built="target/release/demiurg-convert$exe"
    fi
    install -Dm755 "$built" \
        "$STAGE/demiurg_export/bin/$host_tag/demiurg-convert$exe"
    echo "  bundled $host_tag"
    bundled_tags=("$host_tag")
fi

if [[ "$cross_windows" == 1 ]]; then
    # Cross-compile to x86_64-pc-windows-gnu. The target's std comes from the
    # pinned toolchain (rust-toolchain.toml lists it), the linker from
    # mingw-w64, and `libpthread.a` from the mingw pthreads package — Rust's
    # windows-gnu std links it by that exact name, which nixpkgs ships only in
    # `windows.pthreads`. Everything else is statically linked, so the .exe
    # imports nothing but system DLLs and needs no runtime redistributable.
    command -v nix >/dev/null || { echo "--cross windows needs nix" >&2; exit 1; }
    echo "cross-building demiurg-convert for windows-x86_64"
    pthreads=$(nix build --no-link --print-out-paths 'nixpkgs#pkgsCross.mingwW64.windows.pthreads')
    nix shell nixpkgs#pkgsCross.mingwW64.stdenv.cc --command nix develop --command bash -c "
        set -e
        export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc
        export RUSTFLAGS='-L native=$pthreads/lib'
        cargo build --release -p demiurg-convert --target x86_64-pc-windows-gnu
    "
    extra_bins+=("windows-x86_64=target/x86_64-pc-windows-gnu/release/demiurg-convert.exe")
fi

for spec in ${extra_bins[@]+"${extra_bins[@]}"}; do
    tag="${spec%%=*}"
    path="${spec#*=}"
    [[ "$tag" != "$spec" && -f "$path" ]] || { echo "bad --with-bin: $spec" >&2; exit 1; }
    name="demiurg-convert"
    [[ "$tag" == windows-* ]] && name="$name.exe"
    install -Dm755 "$path" "$STAGE/demiurg_export/bin/$tag/$name"
    echo "  bundled $tag"
    bundled_tags+=("$tag")
done

if [[ ${#bundled_tags[@]} -eq 0 ]]; then
    echo "no converter bundled: pass --with-bin, or drop --no-host-build" >&2
    exit 1
fi

# Name the zip after what is actually in it: one platform gets a suffix so a
# folder of releases is readable, several get none — that zip runs anywhere.
if [[ ${#bundled_tags[@]} -eq 1 ]]; then
    zip_path="$DIST/demiurg_export-$version-${bundled_tags[0]}.zip"
else
    zip_path="$DIST/demiurg_export-$version.zip"
fi
rm -f "$zip_path"
# `-X` drops the extra file attributes; the executable bit is set at install
# time by `bundled_converter()` anyway, since not every unzip route keeps it.
(cd "$STAGE" && zip -qr -X "../../$zip_path" demiurg_export)
rm -rf "$STAGE"

echo
echo "wrote $zip_path ($(du -h "$zip_path" | cut -f1))"
echo "install it with Blender's Edit > Preferences > Add-ons > Install from Disk"
