{
  description = "demiurg — voxel asset editor for roxlap / monada (kv6 models, kfa animations, voxel-video)";

  inputs = {
    nixpkgs.url = "flake:nixpkgs";
    # Pinned nightly Rust comes from rust-overlay, driven by
    # rust-toolchain.toml. demiurg inherits roxlap's wasm-threads
    # toolchain requirements (`-Z build-std` + `rust-src`) because
    # demiurg-web (M3) reuses roxlap's wasm-bindgen-rayon path.
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, rust-overlay }:
    let
      forAllSystems = f:
        nixpkgs.lib.genAttrs [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ]
          (system: f {
            pkgs = import nixpkgs {
              inherit system;
              overlays = [ rust-overlay.overlays.default ];
            };
          });
    in {
      devShells = forAllSystems ({ pkgs }:
        let
          # Runtime libs the editor viewport dlopens on Linux: winit +
          # softbuffer (CPU present) and roxlap-gpu's wgpu (Vulkan ICD
          # loader). Needed from M1 onward when demiurg-app opens a
          # window; harmless to ship now. macOS uses Cocoa/Metal and
          # needs none.
          linuxRuntimeLibs = with pkgs; [
            libxkbcommon
            wayland
            libx11
            libxcursor
            libxi
            libxrandr
            libxcb
            vulkan-loader
          ];

          # Single source of truth: the same rust-toolchain.toml cargo
          # reads. Bundles rust-src (for `-Z build-std`) and the
          # wasm32-unknown-unknown target.
          rustToolchain =
            pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
        in {
          default = pkgs.mkShell {
            packages = with pkgs; [
              rustToolchain
              pkg-config
              # wasm32 needs an LLD-class linker; nixpkgs rustc doesn't
              # bundle rust-lld, so provide the system one.
              lld
              # demiurg-web (M3) browser build: wasm-bindgen-cli emits the
              # JS shim, trunk is the dev-server / bundler, Node runs the
              # wasm test harness.
              wasm-bindgen-cli
              nodejs
              trunk
            ] ++ pkgs.lib.optionals pkgs.stdenv.isLinux linuxRuntimeLibs;

            # mkShell only sets PATH / PKG_CONFIG_PATH; the dlopen'd render
            # libs need an explicit search path. macOS skips this.
            shellHook = pkgs.lib.optionalString pkgs.stdenv.isLinux ''
              export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath linuxRuntimeLibs}:''${LD_LIBRARY_PATH:-}"
            '';
          };
        });

      formatter = forAllSystems ({ pkgs }: pkgs.nixpkgs-fmt);
    };
}
