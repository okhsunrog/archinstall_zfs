FROM archlinux:latest

RUN pacman -Syu --noconfirm && \
    pacman -S --noconfirm \
        base-devel git pkgconf clang \
        libxkbcommon libinput libdrm freetype2 fontconfig ttf-dejavu && \
    rm -rf /var/cache/pacman/pkg/*

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | \
    sh -s -- -y --default-toolchain stable --profile minimal && \
    /root/.cargo/bin/rustup component add clippy rustfmt

ENV PATH="/root/.cargo/bin:$PATH"
