# Vendored aur-depends compatibility patch

This directory is based on `aur-depends` 5.0.0. It is vendored temporarily
because the published crate and upstream `master` still require `raur` 7,
which keeps `reqwest` 0.11 and its obsolete Rustls stack in the installer.

Local changes are intentionally limited to dependency metadata:

- `raur` 7 to 8;
- current compatible ALPM and support-crate versions;
- Rustls instead of native TLS as the default transport;
- `publish = false`.

Remove this directory and return to the crates.io dependency after upstream
publishes a release using `raur` 8 or newer.
