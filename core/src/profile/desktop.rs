use super::{DesktopProfile, DisplayManager, DisplayServer, OptionalPackage, Profile, ProfileKind};

/// Helper to keep desktop profile literals concise.
#[allow(clippy::too_many_arguments)]
fn desktop(
    name: &'static str,
    display_name: &'static str,
    packages: Vec<&'static str>,
    display_server: DisplayServer,
    default_display_manager: Option<DisplayManager>,
    needs_seat_access: bool,
    sddm_session: Option<&'static str>,
    optional_packages: Vec<OptionalPackage>,
) -> Profile {
    Profile {
        name,
        display_name,
        description: "",
        packages,
        excluded_packages: Vec::new(),
        services: Vec::new(),
        user_services: Vec::new(),
        post_install_steps: Vec::new(),
        kind: ProfileKind::Desktop(DesktopProfile {
            display_server,
            default_display_manager,
            needs_seat_access,
            sddm_session,
            optional_packages,
        }),
    }
}

/// The tray applets a window manager does not bring itself: without them
/// Wi-Fi, Bluetooth and volume can only be managed from a terminal.
fn tray_opts() -> Vec<OptionalPackage> {
    opts(&[
        ("network-manager-applet", "Network tray applet (nm-applet)"),
        ("blueman", "Bluetooth manager and tray applet"),
        ("pavucontrol", "Volume control"),
    ])
}

/// `base` followed by the tray applets.
fn with_tray(mut base: Vec<OptionalPackage>) -> Vec<OptionalPackage> {
    base.extend(tray_opts());
    base
}

/// Convenience: build a `Vec<OptionalPackage>` from `(package, description)`
/// pairs. Pass an empty description to leave it blank.
fn opts(pkgs: &[(&'static str, &'static str)]) -> Vec<OptionalPackage> {
    pkgs.iter()
        .map(|(p, d)| OptionalPackage::with_desc(p, d))
        .collect()
}

pub fn desktop_profiles() -> Vec<Profile> {
    use DisplayManager::*;
    use DisplayServer::*;

    vec![
        // Bare X server with no DE/WM/DM. Use this when you want to install
        // X yourself or pair it with a window manager that isn't packaged
        // here. Mirrors upstream archinstall's `Xorg` top-level profile.
        desktop(
            "xorg",
            "Xorg (bare)",
            vec!["xorg-server", "xorg-xinit"],
            Xorg,
            None,
            false,
            None,
            Vec::new(),
        ),
        desktop(
            "gnome",
            "GNOME",
            vec!["gnome", "gnome-tweaks"],
            Both,
            Some(Gdm),
            false,
            None,
            opts(&[
                (
                    "gnome-extra",
                    "Extra GNOME apps (Maps, Weather, Calendar, …)",
                ),
                ("gnome-software", "Software centre with Flatpak support"),
                ("flatpak", "Sandboxed application runtime"),
            ]),
        ),
        // The plasma group is the complete desktop: network and Bluetooth
        // applets, volume control, power management, the SDDM settings
        // module, Discover. A hand-picked list used to leave all of those out.
        desktop(
            "kde",
            "KDE Plasma",
            vec!["plasma", "konsole", "kate", "dolphin", "ark"],
            Both,
            Some(Sddm),
            false,
            Some("plasma"),
            opts(&[
                (
                    "kde-applications",
                    "Full KDE application suite (Okular, Gwenview, …)",
                ),
                ("flatpak", "Sandboxed application runtime"),
            ]),
        )
        .excluding(&[
            // A second login manager beside SDDM.
            "plasma-login-manager",
            // A TV interface, developer tools, remote desktop, tablet setup.
            "plasma-bigscreen",
            "plasma-sdk",
            "krdp",
            "wacomtablet",
        ]),
        desktop(
            "xfce",
            "Xfce",
            vec![
                "xfce4",
                "xfce4-goodies",
                "network-manager-applet",
                "blueman",
                "pavucontrol",
                "gvfs",
                "xarchiver",
            ],
            Xorg,
            Some(Lightdm),
            false,
            None,
            opts(&[
                ("thunar", "File manager"),
                ("mousepad", "Lightweight text editor"),
                ("ristretto", "Image viewer"),
            ]),
        ),
        desktop(
            "cinnamon",
            "Cinnamon",
            vec![
                "cinnamon",
                "system-config-printer",
                "gnome-keyring",
                "gnome-terminal",
                "engrampa",
                "gnome-screenshot",
                "gvfs-smb",
                "xed",
                "xdg-user-dirs-gtk",
                "blueman",
            ],
            Xorg,
            Some(Lightdm),
            false,
            None,
            Vec::new(),
        ),
        desktop(
            "budgie",
            "Budgie",
            vec![
                "materia-gtk-theme",
                "budgie",
                "mate-terminal",
                "nemo",
                "papirus-icon-theme",
            ],
            Xorg,
            Some(Lightdm),
            false,
            None,
            Vec::new(),
        ),
        desktop(
            "mate",
            "MATE",
            vec!["mate", "mate-extra", "network-manager-applet", "blueman"],
            Xorg,
            Some(Lightdm),
            false,
            None,
            Vec::new(),
        ),
        desktop(
            "deepin",
            "Deepin",
            vec!["deepin", "deepin-terminal", "deepin-editor"],
            Xorg,
            Some(Lightdm),
            false,
            None,
            Vec::new(),
        ),
        desktop(
            "lxqt",
            "LXQt",
            vec![
                "lxqt",
                "breeze-icons",
                "oxygen-icons",
                "xdg-utils",
                "gnu-free-fonts",
                "l3afpad",
                "slock",
                "network-manager-applet",
                "blueman",
            ],
            Xorg,
            Some(Sddm),
            false,
            Some("lxqt"),
            Vec::new(),
        ),
        desktop(
            "hyprland",
            "Hyprland",
            vec![
                "hyprland",
                "waybar",
                "dunst",
                "kitty",
                "uwsm",
                "dolphin",
                // Hyprland's first-run screen offers its own launcher as the
                // default; shipping it means nothing in that screen is red.
                "hyprlauncher",
                "xdg-desktop-portal-hyprland",
                "qt5-wayland",
                "qt6-wayland",
                // The Hyprland project's own agent, which unlike
                // polkit-kde-agent ships a user unit and so actually runs
                // without the user writing an exec-once for it.
                "hyprpolkitagent",
                "grim",
                "slurp",
            ],
            Wayland,
            Some(Sddm),
            true,
            Some("hyprland"),
            with_tray(opts(&[
                ("hyprpaper", "Wallpaper utility from the Hyprland project"),
                ("hypridle", "Idle daemon (auto-lock, dim, sleep)"),
                ("hyprlock", "Screen locker"),
                ("awww", "Animated wallpaper daemon (formerly swww)"),
                ("mako", "Wayland notification daemon"),
                ("wofi", "Application launcher, in place of hyprlauncher"),
                ("wl-clipboard", "Clipboard helper (wl-copy / wl-paste)"),
            ])),
        )
        // Both ship a user unit wanted by graphical-session.target, so the
        // session brings them up without the user writing an exec-once. A
        // bar that is installed and never starts is a bar the user does not
        // have.
        .with_user_services(&["hyprpolkitagent", "waybar"]),
        desktop(
            "sway",
            "Sway",
            vec![
                "sway",
                "swaybg",
                "swaylock",
                "swayidle",
                "waybar",
                "wmenu",
                "brightnessctl",
                "grim",
                "slurp",
                "pavucontrol",
                "foot",
                "xorg-xwayland",
            ],
            Wayland,
            Some(Lightdm),
            true,
            Some("sway"),
            with_tray(opts(&[
                ("wl-clipboard", "Clipboard helper (wl-copy / wl-paste)"),
                ("mako", "Wayland notification daemon"),
            ])),
        ),
        desktop(
            "i3",
            "i3",
            vec![
                "i3-wm",
                "i3lock",
                "i3status",
                "i3blocks",
                "xss-lock",
                "xterm",
                "lightdm-gtk-greeter",
                "dmenu",
            ],
            Xorg,
            Some(Lightdm),
            false,
            Some("i3"),
            with_tray(opts(&[
                ("polybar", "Modular status bar"),
                ("rofi", "Application launcher and dmenu replacement"),
                ("feh", "Image viewer often used to set wallpaper"),
            ])),
        ),
        desktop(
            "cosmic",
            "COSMIC",
            vec!["cosmic", "xdg-user-dirs"],
            Wayland,
            Some(CosmicGreeter),
            false,
            None,
            Vec::new(),
        ),
        desktop(
            "enlightenment",
            "Enlightenment",
            vec![
                "enlightenment",
                "terminology",
                "lightdm-gtk-greeter",
                "xdg-user-dirs",
            ],
            Xorg,
            Some(Lightdm),
            false,
            None,
            tray_opts(),
        ),
        desktop(
            "awesome",
            "Awesome",
            vec![
                "awesome",
                "xterm",
                "lightdm-gtk-greeter",
                "dmenu",
                "picom",
                "xdg-user-dirs",
            ],
            Xorg,
            Some(Lightdm),
            false,
            None,
            tray_opts(),
        ),
        desktop(
            "bspwm",
            "Bspwm",
            vec![
                "bspwm",
                "sxhkd",
                "xterm",
                "lightdm-gtk-greeter",
                "dmenu",
                "picom",
                "xdg-user-dirs",
            ],
            Xorg,
            Some(Lightdm),
            false,
            None,
            tray_opts(),
        ),
        desktop(
            "labwc",
            "LabWC",
            vec![
                "labwc",
                "waybar",
                "foot",
                "fuzzel",
                "xdg-desktop-portal-wlr",
                "xdg-user-dirs",
            ],
            Wayland,
            Some(Sddm),
            true,
            Some("labwc"),
            tray_opts(),
        ),
        desktop(
            "niri",
            "Niri",
            vec![
                "niri",
                "foot",
                "fuzzel",
                "waybar",
                "xdg-desktop-portal-gnome",
                "xwayland-satellite",
                "xdg-user-dirs",
            ],
            Wayland,
            Some(Sddm),
            true,
            Some("niri"),
            tray_opts(),
        ),
        desktop(
            "qtile",
            "Qtile",
            vec![
                "qtile",
                "xterm",
                "lightdm-gtk-greeter",
                "dmenu",
                "xdg-user-dirs",
            ],
            Xorg,
            Some(Lightdm),
            false,
            None,
            tray_opts(),
        ),
        desktop(
            "river",
            "River",
            vec![
                "river",
                "foot",
                "fuzzel",
                "waybar",
                "xdg-desktop-portal-wlr",
                "xdg-user-dirs",
            ],
            Wayland,
            Some(Sddm),
            true,
            Some("river"),
            tray_opts(),
        ),
        desktop(
            "xmonad",
            "XMonad",
            vec![
                "xmonad",
                "xmonad-contrib",
                "xterm",
                "lightdm-gtk-greeter",
                "dmenu",
                "xdg-user-dirs",
            ],
            Xorg,
            Some(Lightdm),
            false,
            None,
            tray_opts(),
        ),
    ]
}
