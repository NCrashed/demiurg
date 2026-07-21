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
          (system:
            let
              pkgs = import nixpkgs {
                inherit system;
                overlays = [ rust-overlay.overlays.default ];
              };

              # Runtime libs the editor viewport dlopens on Linux: winit +
              # softbuffer (CPU present) and roxlap-gpu's wgpu (Vulkan ICD
              # loader). Needed whenever demiurg-app opens a window; macOS
              # uses Cocoa/Metal and needs none.
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
            in
            f { inherit pkgs linuxRuntimeLibs rustToolchain; });
    in {
      packages = forAllSystems ({ pkgs, linuxRuntimeLibs, rustToolchain }:
        let
          # Build with the pinned toolchain (roxlap 0.30 needs rustc 1.92+;
          # our nightly clears it) instead of nixpkgs' default rustc.
          rustPlatform = pkgs.makeRustPlatform {
            cargo = rustToolchain;
            rustc = rustToolchain;
          };

          demiurg = rustPlatform.buildRustPackage {
            pname = "demiurg";
            version = "0.12.1";
            src = ./.;
            cargoLock.lockFile = ./Cargo.lock;

            # Only the editor binary crate (the workspace has no other bins,
            # and this skips building the libs' test-only deps).
            cargoBuildFlags = [ "-p" "demiurg-app" ];

            nativeBuildInputs = [ pkgs.pkg-config pkgs.makeWrapper ];
            buildInputs =
              pkgs.lib.optionals pkgs.stdenv.isLinux linuxRuntimeLibs;

            # Tests want a display / GPU; the CI job runs them separately.
            doCheck = false;

            # The pinned nightly rustc overflows its default stack in borrowck
            # while compiling wayland-protocols under opt-level=3, crashing with
            # SIGSEGV. Give the compiler a bigger stack (its own ICE hint).
            RUST_MIN_STACK = "16777216";

            # The render libs are dlopen'd at runtime, so an rpath can't
            # reach them — wrap the binary with an explicit library search
            # path. macOS links Metal/Cocoa directly and needs no wrapper.
            postInstall = pkgs.lib.optionalString pkgs.stdenv.isLinux ''
              wrapProgram $out/bin/demiurg \
                --prefix LD_LIBRARY_PATH : "${pkgs.lib.makeLibraryPath linuxRuntimeLibs}"
            '';

            meta = {
              description =
                "Voxel asset editor for roxlap (kv6 models, kfa animations, voxel-clips)";
              homepage = "https://github.com/NCrashed/demiurg";
              license = with pkgs.lib.licenses; [ mit asl20 ];
              mainProgram = "demiurg";
            };
          };
        in {
          default = demiurg;
          demiurg = demiurg;
        });

      apps = forAllSystems ({ pkgs, ... }: {
        default = {
          type = "app";
          program = "${self.packages.${pkgs.stdenv.hostPlatform.system}.default}/bin/demiurg";
        };
      });

      devShells = forAllSystems ({ pkgs, linuxRuntimeLibs, rustToolchain }: {
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

      formatter = forAllSystems ({ pkgs, ... }: pkgs.nixpkgs-fmt);
    };
}
