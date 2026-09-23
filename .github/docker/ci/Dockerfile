FROM archlinux@sha256:b0deabeb3d283da2c7f7dbf0eea051b7b2cd0554e0b737cc457fd21683bdcdd1

# Rust build deps: base-devel, pkgconf, clang for C-FFI crates.
# UI libs: libxkbcommon/libinput/seatd(libseat)/libdrm/freetype2/fontconfig/ttf-dejavu for the slint UI
#   (ttf-dejavu is also what the ISO ships, so headless captures use the same fonts).
# python/uv run the UI review scripts in the ui-review job.
# ISO build deps: archiso (mkarchiso), pacman-contrib (paccache), rsync, sudo, just
#   — installed here so build.yml ISO jobs can reuse this image without pacman -S.
RUN pacman-key --init && \
    pacman-key --populate archlinux && \
    pacman -Syu --noconfirm && \
    pacman -S --noconfirm \
        base-devel git pkgconf clang \
        libxkbcommon libinput seatd libdrm freetype2 fontconfig ttf-dejavu \
        python uv \
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
