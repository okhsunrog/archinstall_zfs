# Local security patch

This directory is based on `wayland-scanner` 0.31.10. That release requires
`quick-xml` 0.39, which is affected by RUSTSEC-2026-0194 and
RUSTSEC-2026-0195.

The local patch contains only the parser migration required for `quick-xml`
0.41. Using the entire unreleased `wayland-scanner` branch is not compatible
with the published `wayland-client` and `wayland-backend` 0.31 releases because
it also changes generated protocol APIs.

Remove this directory and the workspace patch after a compatible
`wayland-scanner` release with `quick-xml` 0.41 or newer is published.
