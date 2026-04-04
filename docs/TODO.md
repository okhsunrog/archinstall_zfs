# TODO — Missing Features

Compared against archinstall (Python). Excludes non-ZFS filesystems and non-ZFSBootMenu bootloaders.

---

## High Priority

### GPU Driver Detection & Installation

Auto-detect GPU and install appropriate drivers. Without this, desktop profiles are broken on NVIDIA (Wayland won't start).

- [x] Detect GPU vendor from `lspci` (`system::gpu::detect_gpus`)
- [x] Map vendor to package sets (`GfxDriver::packages()` — mesa, vulkan-radeon, nvidia-open-dkms, etc.)
- [x] Auto-suggest driver from detected hardware (`system::gpu::suggested_driver`)
- [x] Integrate with profile system — `gfx_driver: Option<GfxDriver>` in config, installed in `install_profile`
- [x] Handle multi-GPU setups (Intel+NVIDIA → `AllOpenSource`)
- [x] TUI selection for driver preference (`pick_gpu_driver` in pickers.rs — detects GPUs via lspci, marks suggested driver with ✦)

### WiFi Configuration During Install

Installer is unusable on laptops without ethernet.

- [x] Detect wireless interfaces (`system::wifi::detect_wifi_interfaces`)
- [x] Scan networks via `iwctl` (`system::wifi::scan_networks`)
- [x] Connection via `iwctl --passphrase` (`system::wifi::connect`)
- [x] Connection verification (`system::wifi::check_connected`)
- [x] Copy WiFi config to target (handled by `network::copy_iso_network` — iwd saves profiles to `/var/lib/iwd`)
- [x] TUI network selection screen with signal strength (`screens/wifi.rs`, shown before wizard on first launch)
- [x] TUI password prompt for secured networks (masked `run_edit` popup)

### X11/Wayland Keyboard Layout

Currently only console keymap (vconsole.conf) is set. Graphical sessions need X11/Wayland keymap too.

- [x] Add `x11_variant` to config
- [x] Write `/etc/X11/xorg.conf.d/00-keyboard.conf` (`locale::set_x11_keyboard`) — uses `keyboard_layout` + optional `x11_variant`
- [ ] TUI: variant selection field
- [x] Handle layout variants (e.g. `us` with `intl` variant)

---

## Medium Priority

### More Desktop/WM Profiles

Current: GNOME, KDE Plasma, Xfce, Cinnamon, Budgie, MATE, Deepin, LXQt, Hyprland, Sway, i3, Cosmic.

Missing WMs:
- [x] Awesome
- [x] Bspwm
- [x] Enlightenment
- [x] LabWC
- [x] Niri
- [x] Qtile
- [x] River
- [x] XMonad

Missing servers:
- [x] Lighttpd
- [x] Tomcat

Profile system improvements:
- [ ] Display manager / greeter selection per profile (GDM, SDDM, LightDM, Ly)
- [ ] Profile post-install hooks (e.g. enable PipeWire user services)
- [ ] Per-profile recommended vs optional packages

### User Management

Basic user creation works. Missing:

- [x] SSH public key setup (`~user/.ssh/authorized_keys`) — `ssh_authorized_keys: Vec<String>` in `UserConfig`
- [x] Auto-login configuration per display manager — GDM, SDDM, LightDM, Ly (`autologin: bool` in `UserConfig`)
- [ ] Password strength feedback in TUI

### Post-Install Customization

- [x] Custom chroot commands (`post_install_commands: Vec<String>` config field, run via `sh -c` in chroot)
- [x] SSD detection + ZFS-native TRIM: NVMe → `autotrim=on`, SATA SSD → `zfs-trim-weekly@<pool>.timer` (`fstrim.timer` is not used — it silently skips ZFS pools)
- [ ] Service enable verification (report warnings on failure)

---

## Low Priority

### Security

- [ ] Optional firewall setup (ufw): default deny incoming, allow SSH if enabled
- [ ] Encrypted passwords in config (yescrypt hash instead of plaintext)
- [ ] Secure Boot — blocked on upstream ZFSBootMenu support

### Internationalization

- [ ] Choose i18n framework (fluent-rs, gettext, or key-value)
- [ ] Extract all user-facing strings from TUI
- [ ] Language selection at startup
- [ ] Start with 3-5 languages: English, Russian, German, Spanish, Chinese
