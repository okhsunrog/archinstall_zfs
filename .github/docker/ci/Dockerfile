FROM archlinux:latest

RUN pacman -Syu --noconfirm && \
    pacman -S --noconfirm \
        base-devel git pkgconf clang \
        libxkbcommon libinput libdrm freetype2 fontconfig ttf-dejavu && \
    rm -rf /var/cache/pacman/pkg/*

ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | \
    sh -s -- -y --default-toolchain stable --profile minimal \
              --no-modify-path && \
    rustup component add clippy rustfmt && \
    chmod -R a+w /usr/local/rustup /usr/local/cargo
