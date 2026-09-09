# Repository instructions

Read and follow [AGENTS.md](AGENTS.md) first. It is the shared source of project
rules; keep those rules there rather than maintaining a second conflicting copy.

Before development, read [docs/development.md](docs/development.md).
For any Slint/UI implementation or review, read
[docs/slint-ui-review.md](docs/slint-ui-review.md) and use the real preview and
interaction scripts described in [slint-ui/README.md](slint-ui/README.md).

Do not call UI work complete from compilation, passing assertions or a contact
sheet alone. Review individual screenshots, interaction/state coverage and the
design itself. Use Full HD at 100% as the primary baseline, then compact and
scaled configurations. Keep preview, VM and hardware claims separate.

For binary-only USB updates, follow
[gen_iso/LIVE_UPDATE.md](gen_iso/LIVE_UPDATE.md); rebuild the base ISO when its
service, kernel or system dependencies change. Never ship a desktop-mock/MCP
preview executable as the production installer.
