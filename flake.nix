{
  description = "archinstall-zfs dev shell (cross-distro Rust build environment)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
      in {
        devShells.default = pkgs.mkShell {
          # Rust toolchain + build tools
          nativeBuildInputs = with pkgs; [
            rustc cargo clippy rustfmt rust-analyzer
            pkg-config
            clang
            just
          ];

          # Native libraries linked by the workspace:
          #   pacman              -> libalpm (required by the alpm / alpm-sys crates)
          #   libxkbcommon,
          #   libinput,
          #   libdrm, mesa,
          #   freetype,
          #   fontconfig,
          #   wayland, udev       -> slint UI backend (gbm-sys, input-sys,
          #                         libudev-sys, wayland-sys, yeslogic-fontconfig-sys)
          buildInputs = with pkgs; [
            pacman
            libxkbcommon
            libinput
            libdrm
            mesa            # provides libgbm for gbm-sys
            freetype
            fontconfig
            wayland
            wayland-protocols
            systemdLibs     # provides libudev for libudev-sys
          ];

          # Some crates locate libs through LIBCLANG_PATH / BINDGEN_EXTRA_CLANG_ARGS;
          # set them preemptively so bindgen works without extra host config.
          LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";

          shellHook = ''
            echo "archinstall-zfs devshell: rustc $(rustc --version | awk '{print $2}'), pacman $(pacman --version | head -n1 | awk '{print $3}')"
            echo "Native cargo works here. For ISO builds run 'just iso-test-podman' or 'just iso-full-podman'."
          '';
        };
      });
}
