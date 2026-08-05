FROM archlinux@sha256:345a872f6c95e082d4b8c050af637eebb57402c6e2177b411c3acf7df84eb33b

# Rust build deps: base-devel, pkgconf, clang for C-FFI crates.
# UI libs: libxkbcommon/libinput/libdrm/freetype2/fontconfig/ttf-dejavu for the slint UI.
# ISO build deps: archiso (mkarchiso), pacman-contrib (paccache), rsync, sudo, just
#   — installed here so build.yml ISO jobs can reuse this image without pacman -S.
RUN pacman-key --init && \
    pacman-key --populate archlinux && \
    pacman -Syu --noconfirm && \
    pacman -S --noconfirm \
        base-devel git pkgconf clang \
        libxkbcommon libinput libdrm freetype2 fontconfig ttf-dejavu \
        archiso just pacman-contrib rsync sudo && \
    pacman -Scc --noconfirm && \
    rm -rf /var/cache/pacman/pkg/*

ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | \
    sh -s -- -y --default-toolchain 1.97.1 --profile minimal \
              --no-modify-path && \
    rustup component add --toolchain 1.97.1 clippy rustfmt && \
    cargo install cargo-audit --version 0.22.2 --locked && \
    chmod -R a+w /usr/local/rustup /usr/local/cargo
