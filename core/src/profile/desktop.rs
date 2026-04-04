use super::Profile;

pub fn desktop_profiles() -> Vec<Profile> {
    vec![
        Profile {
            name: "gnome",
            display_name: "GNOME",
            packages: vec!["gnome", "gnome-tweaks"],
            services: vec!["gdm"],
            optional_packages: vec!["gnome-extra", "gnome-software", "flatpak"],
            ..Profile::default()
        },
        Profile {
            name: "kde",
            display_name: "KDE Plasma",
            packages: vec![
                "plasma-desktop",
                "konsole",
                "kate",
                "dolphin",
                "ark",
                "plasma-workspace",
            ],
            services: vec!["sddm"],
            optional_packages: vec!["kde-applications", "flatpak", "discover"],
            ..Profile::default()
        },
        Profile {
            name: "xfce",
            display_name: "Xfce",
            packages: vec!["xfce4", "xfce4-goodies", "pavucontrol", "gvfs", "xarchiver"],
            services: vec!["lightdm"],
            optional_packages: vec!["thunar", "mousepad", "ristretto"],
            ..Profile::default()
        },
        Profile {
            name: "cinnamon",
            display_name: "Cinnamon",
            packages: vec![
                "cinnamon",
                "system-config-printer",
                "gnome-keyring",
                "gnome-terminal",
                "engrampa",
                "gnome-screenshot",
                "gvfs-smb",
                "xed",
                "xdg-user-dirs-gtk",
            ],
            services: vec!["lightdm"],
            ..Profile::default()
        },
        Profile {
            name: "budgie",
            display_name: "Budgie",
            packages: vec![
                "materia-gtk-theme",
                "budgie",
                "mate-terminal",
                "nemo",
                "papirus-icon-theme",
            ],
            services: vec!["lightdm"],
            ..Profile::default()
        },
        Profile {
            name: "mate",
            display_name: "MATE",
            packages: vec!["mate", "mate-extra"],
            services: vec!["lightdm"],
            ..Profile::default()
        },
        Profile {
            name: "deepin",
            display_name: "Deepin",
            packages: vec!["deepin", "deepin-terminal", "deepin-editor"],
            services: vec!["lightdm"],
            ..Profile::default()
        },
        Profile {
            name: "lxqt",
            display_name: "LXQt",
            packages: vec![
                "lxqt",
                "breeze-icons",
                "oxygen-icons",
                "xdg-utils",
                "ttf-freefont",
                "l3afpad",
                "slock",
            ],
            services: vec!["sddm"],
            ..Profile::default()
        },
        Profile {
            name: "hyprland",
            display_name: "Hyprland",
            packages: vec![
                "hyprland",
                "dunst",
                "kitty",
                "uwsm",
                "dolphin",
                "wofi",
                "xdg-desktop-portal-hyprland",
                "qt5-wayland",
                "qt6-wayland",
                "polkit-kde-agent",
                "grim",
                "slurp",
            ],
            services: vec!["sddm"],
            needs_seat_access: true,
            optional_packages: vec![
                "hyprpaper",
                "hypridle",
                "hyprlock",
                "swww",
                "mako",
                "wl-clipboard",
            ],
            ..Profile::default()
        },
        Profile {
            name: "sway",
            display_name: "Sway",
            packages: vec![
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
            services: vec!["lightdm"],
            needs_seat_access: true,
            optional_packages: vec!["swaylock-effects", "wl-clipboard", "mako"],
            ..Profile::default()
        },
        Profile {
            name: "i3",
            display_name: "i3",
            packages: vec![
                "i3-wm",
                "i3lock",
                "i3status",
                "i3blocks",
                "xss-lock",
                "xterm",
                "lightdm-gtk-greeter",
                "lightdm",
                "dmenu",
            ],
            services: vec!["lightdm"],
            optional_packages: vec!["polybar", "rofi", "feh", "nitrogen"],
            ..Profile::default()
        },
        Profile {
            name: "cosmic",
            display_name: "COSMIC",
            packages: vec!["cosmic", "xdg-user-dirs"],
            services: vec!["cosmic-greeter"],
            ..Profile::default()
        },
        Profile {
            name: "enlightenment",
            display_name: "Enlightenment",
            packages: vec![
                "enlightenment",
                "terminology",
                "lightdm",
                "lightdm-gtk-greeter",
                "xdg-user-dirs",
            ],
            services: vec!["lightdm"],
            ..Profile::default()
        },
        Profile {
            name: "awesome",
            display_name: "Awesome",
            packages: vec![
                "awesome",
                "xterm",
                "lightdm",
                "lightdm-gtk-greeter",
                "dmenu",
                "picom",
                "xdg-user-dirs",
            ],
            services: vec!["lightdm"],
            ..Profile::default()
        },
        Profile {
            name: "bspwm",
            display_name: "Bspwm",
            packages: vec![
                "bspwm",
                "sxhkd",
                "xterm",
                "lightdm",
                "lightdm-gtk-greeter",
                "dmenu",
                "picom",
                "xdg-user-dirs",
            ],
            services: vec!["lightdm"],
            ..Profile::default()
        },
        Profile {
            name: "labwc",
            display_name: "LabWC",
            packages: vec![
                "labwc",
                "waybar",
                "foot",
                "fuzzel",
                "xdg-desktop-portal-wlr",
                "xdg-user-dirs",
                "sddm",
            ],
            services: vec!["sddm"],
            needs_seat_access: true,
            ..Profile::default()
        },
        Profile {
            name: "niri",
            display_name: "Niri",
            packages: vec![
                "niri",
                "foot",
                "fuzzel",
                "waybar",
                "xdg-desktop-portal-gnome",
                "xwayland-satellite",
                "xdg-user-dirs",
                "sddm",
            ],
            services: vec!["sddm"],
            needs_seat_access: true,
            ..Profile::default()
        },
        Profile {
            name: "qtile",
            display_name: "Qtile",
            packages: vec![
                "qtile",
                "xterm",
                "lightdm",
                "lightdm-gtk-greeter",
                "dmenu",
                "xdg-user-dirs",
            ],
            services: vec!["lightdm"],
            ..Profile::default()
        },
        Profile {
            name: "river",
            display_name: "River",
            packages: vec![
                "river",
                "foot",
                "fuzzel",
                "waybar",
                "xdg-desktop-portal-wlr",
                "xdg-user-dirs",
                "sddm",
            ],
            services: vec!["sddm"],
            needs_seat_access: true,
            ..Profile::default()
        },
        Profile {
            name: "xmonad",
            display_name: "XMonad",
            packages: vec![
                "xmonad",
                "xmonad-contrib",
                "xterm",
                "lightdm",
                "lightdm-gtk-greeter",
                "dmenu",
                "xdg-user-dirs",
            ],
            services: vec!["lightdm"],
            ..Profile::default()
        },
    ]
}
