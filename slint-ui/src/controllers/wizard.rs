//! Wizard controller: rebuild items on step change, dispatch
//! item-activation/toggle/select/text/keyboard-nav callbacks, and the
//! show_select / show_text_input / show_*_popup helpers used by
//! handle_item_activated.

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use archinstall_zfs_core::config::types::GlobalConfig;

use crate::config_items::{apply_radio, apply_text, build_step_items, next_selectable_index};
use crate::controllers::welcome::KernelScan;
use crate::refresh::refresh_items;
use crate::ui::{App, EditingState, ItemType, PopupState, SelectOption, Theme, WizardState};

pub fn setup(app: &App, config: &Rc<RefCell<GlobalConfig>>, kernel_scan: &KernelScan) {
    setup_step_changed(app, config);
    setup_item_activated(app, config, kernel_scan);
    setup_toggle(app, config);
    setup_select_confirmed(app, config, kernel_scan);
    setup_text_confirmed(app, config);
    setup_keyboard_nav(app, config);
    setup_select_filter(app);
    setup_password_strength(app);
}

fn setup_step_changed(app: &App, config: &Rc<RefCell<GlobalConfig>>) {
    let weak = app.as_weak();
    let cfg = config.clone();
    app.global::<WizardState>().on_step_changed(move |_step| {
        let Some(app) = weak.upgrade() else { return };
        refresh_items(&app, &cfg.borrow());
    });
}

fn setup_item_activated(app: &App, config: &Rc<RefCell<GlobalConfig>>, kernel_scan: &KernelScan) {
    let weak = app.as_weak();
    let cfg = config.clone();
    let kscan = kernel_scan.clone();
    app.on_item_activated(move |key| {
        let Some(app) = weak.upgrade() else { return };

        // Inline radio option clicks: "radio:{group_key}:{index}"
        if let Some(rest) = key.strip_prefix("radio:") {
            if let Some((group_key, idx_str)) = rest.rsplit_once(':')
                && let Ok(idx) = idx_str.parse::<i32>()
            {
                let mut c = cfg.borrow_mut();
                apply_radio(&mut c, group_key, idx);
                refresh_items(&app, &c);
            }
            return;
        }

        handle_item_activated(&app, &key, &cfg.borrow(), &kscan);
    });
}

fn setup_toggle(app: &App, config: &Rc<RefCell<GlobalConfig>>) {
    let weak = app.as_weak();
    let cfg = config.clone();
    app.on_toggle_activated(move |key| {
        let Some(app) = weak.upgrade() else { return };
        let mut c = cfg.borrow_mut();
        match key.as_str() {
            "ntp" => c.ntp = !c.ntp,
            "bluetooth" => c.bluetooth = !c.bluetooth,
            "zrepl" => c.zrepl_enabled = !c.zrepl_enabled,
            _ => return,
        }
        refresh_items(&app, &c);
    });
}

fn setup_select_confirmed(app: &App, config: &Rc<RefCell<GlobalConfig>>, kernel_scan: &KernelScan) {
    let weak = app.as_weak();
    let cfg = config.clone();
    let kscan = kernel_scan.clone();
    app.on_select_confirmed(move |key, idx| {
        let Some(app) = weak.upgrade() else { return };

        if key == "timezone_region" {
            let regions = archinstall_zfs_core::installer::locale::list_timezone_regions();
            if let Some(&region) = regions.get(idx as usize) {
                let cities = archinstall_zfs_core::installer::locale::list_timezone_cities(region);
                let city_strs: Vec<&str> = cities.iter().map(|s| s.as_str()).collect();
                let tz_key = format!("timezone_city:{region}");
                show_select(&app, &tz_key, &format!("{region} /"), &city_strs, 0);
            }
            return;
        }

        if key.starts_with("timezone_city:") {
            let region = key.strip_prefix("timezone_city:").unwrap();
            let cities = archinstall_zfs_core::installer::locale::list_timezone_cities(region);
            if let Some(city) = cities.get(idx as usize) {
                cfg.borrow_mut().timezone = Some(format!("{region}/{city}"));
                refresh_items(&app, &cfg.borrow());
            }
            return;
        }

        if key == "kernel_select" {
            let kernels = archinstall_zfs_core::kernel::AVAILABLE_KERNELS;
            if let Some(info) = kernels.get(idx as usize) {
                let mut c = cfg.borrow_mut();
                c.kernels = Some(vec![info.name.to_string()]);
                let auto_mode = kscan.with(|cached| {
                    cached
                        .and_then(|results| results.get(idx as usize))
                        .and_then(|r| r.best_mode())
                });
                if let Some(mode) = auto_mode {
                    c.zfs_module_mode = mode;
                }
                refresh_items(&app, &c);
            }
            return;
        }

        if key == "locale_select" {
            let selected_text = app
                .global::<PopupState>()
                .get_select_options()
                .row_data(idx as usize);
            if let Some(opt) = selected_text {
                cfg.borrow_mut().locale = Some(opt.text.to_string());
                refresh_items(&app, &cfg.borrow());
            }
            return;
        }

        if key == "keyboard_select" {
            let selected_text = app
                .global::<PopupState>()
                .get_select_options()
                .row_data(idx as usize);
            if let Some(opt) = selected_text {
                cfg.borrow_mut().keyboard_layout = opt.text.to_string();
                refresh_items(&app, &cfg.borrow());
            }
            return;
        }

        let mut c = cfg.borrow_mut();
        apply_radio(&mut c, &key, idx);
        refresh_items(&app, &c);
    });
}

fn setup_text_confirmed(app: &App, config: &Rc<RefCell<GlobalConfig>>) {
    let weak = app.as_weak();
    let cfg = config.clone();
    app.on_text_confirmed(move |key, val| {
        let Some(app) = weak.upgrade() else { return };
        let mut c = cfg.borrow_mut();
        apply_text(&mut c, &key, &val);
        refresh_items(&app, &c);
    });
}

fn setup_keyboard_nav(app: &App, config: &Rc<RefCell<GlobalConfig>>) {
    let weak = app.as_weak();
    let cfg = config.clone();
    app.on_key_nav_down(move || {
        let Some(app) = weak.upgrade() else { return };
        let items = build_step_items(
            app.global::<WizardState>().get_current_step() as usize,
            &cfg.borrow(),
        );
        let current = app.global::<WizardState>().get_focused_index();
        let next = next_selectable_index(&items, current, 1);
        app.global::<WizardState>().set_focused_index(next);
    });

    let weak = app.as_weak();
    let cfg = config.clone();
    app.on_key_nav_up(move || {
        let Some(app) = weak.upgrade() else { return };
        let items = build_step_items(
            app.global::<WizardState>().get_current_step() as usize,
            &cfg.borrow(),
        );
        let current = app.global::<WizardState>().get_focused_index();
        let next = next_selectable_index(&items, current, -1);
        app.global::<WizardState>().set_focused_index(next);
    });

    let weak = app.as_weak();
    let cfg = config.clone();
    app.on_key_nav_activate(move || {
        let Some(app) = weak.upgrade() else { return };
        let idx = app.global::<WizardState>().get_focused_index();
        let items = build_step_items(
            app.global::<WizardState>().get_current_step() as usize,
            &cfg.borrow(),
        );
        if idx < 0 || idx as usize >= items.len() {
            return;
        }
        let item = &items[idx as usize];
        let item_type = item.item_type;
        let key = item.key.clone();
        if item_type == ItemType::Action {
            if key == "install" {
                app.invoke_install_requested();
            } else if key == "quit" {
                let _ = app.window().hide();
            }
        } else if item_type == ItemType::Toggle {
            app.invoke_toggle_activated(key);
        } else if item_type != ItemType::Separator
            && item_type != ItemType::Readonly
            && item_type != ItemType::Warning
            && item_type != ItemType::RadioHeader
        {
            app.invoke_item_activated(key);
        }
    });
}

fn setup_select_filter(app: &App) {
    let weak = app.as_weak();
    app.on_select_filter_changed(move |key, filter_text| {
        let Some(app) = weak.upgrade() else { return };
        let filter = filter_text.to_lowercase();

        if key == "locale_select" {
            let all_locales = archinstall_zfs_core::installer::locale::list_locales();
            let filtered = fuzzy_filter(&all_locales, &filter);
            let popup = app.global::<PopupState>();
            popup.set_select_options(ModelRc::new(VecModel::from(filtered)));
            popup.set_select_index(-1);
        }

        if key == "keyboard_select" {
            let all_keymaps = archinstall_zfs_core::installer::locale::list_keymaps();
            let filtered = fuzzy_filter(&all_keymaps, &filter);
            let popup = app.global::<PopupState>();
            popup.set_select_options(ModelRc::new(VecModel::from(filtered)));
            popup.set_select_index(-1);
        }
    });
}

fn fuzzy_filter(items: &[String], filter: &str) -> Vec<SelectOption> {
    if filter.is_empty() {
        return items
            .iter()
            .map(|s| SelectOption {
                text: SharedString::from(s.as_str()),
            })
            .collect();
    }
    let mut scored: Vec<_> = items
        .iter()
        .filter_map(|s| sublime_fuzzy::best_match(filter, s).map(|m| (m.score(), s)))
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored
        .into_iter()
        .map(|(_, s)| SelectOption {
            text: SharedString::from(s.as_str()),
        })
        .collect()
}

fn setup_password_strength(app: &App) {
    let weak = app.as_weak();
    app.on_text_input_edited(move |key, value| {
        let Some(app) = weak.upgrade() else { return };

        if key != "root_password" && key != "encryption_password" {
            return;
        }
        if value.is_empty() {
            app.global::<PopupState>().set_password_strength_score(-1);
            return;
        }
        let entropy = zxcvbn::zxcvbn(value.as_str(), &[]);
        let score = u8::from(entropy.score());
        let theme = app.global::<Theme>().get_c();
        let (label, color) = match score {
            0 => ("Very weak", theme.red),
            1 => ("Weak", theme.peach),
            2 => ("Fair", theme.yellow),
            3 => ("Strong", theme.green),
            _ => ("Very strong", theme.teal),
        };

        let hint = entropy
            .feedback()
            .and_then(|f| f.suggestions().first().map(|s| s.to_string()))
            .unwrap_or_else(|| {
                let crack_time = entropy
                    .crack_times()
                    .online_no_throttling_10_per_second()
                    .to_string();
                format!("~{crack_time} to crack")
            });

        let popup = app.global::<PopupState>();
        popup.set_password_strength_score(score as i32);
        popup.set_password_strength_label(SharedString::from(label));
        popup.set_password_strength_hint(SharedString::from(hint));
        popup.set_password_strength_color(color);
    });
}

/// Format the cached/fresh kernel scan results as user-facing strings.
fn build_kernel_options(
    results: &[archinstall_zfs_core::kernel::scanner::CompatibilityResult],
) -> Vec<String> {
    archinstall_zfs_core::kernel::AVAILABLE_KERNELS
        .iter()
        .zip(results.iter())
        .map(|(info, result)| {
            let ver = result.kernel_version.as_deref().unwrap_or("?");
            if result.best_mode().is_some() {
                format!(
                    "\u{2713} {} ({}) [{}]",
                    info.display_name,
                    ver,
                    result.mode_label()
                )
            } else {
                format!("\u{2717} {} ({}) [incompatible]", info.display_name, ver)
            }
        })
        .collect()
}

// ── Item activation (open the right popup for the clicked row) ──────

fn handle_item_activated(app: &App, key: &str, config: &GlobalConfig, kernel_scan: &KernelScan) {
    match key {
        "kernel" => {
            // Use the cached scan results if available; otherwise block-scan now.
            let fresh: Vec<archinstall_zfs_core::kernel::scanner::CompatibilityResult>;
            let options =
                if let Some(opts) = kernel_scan.with(|cached| cached.map(build_kernel_options)) {
                    opts
                } else {
                    let rt = tokio::runtime::Handle::current();
                    fresh = rt.block_on(archinstall_zfs_core::kernel::scanner::scan_all_kernels());
                    build_kernel_options(&fresh)
                };

            let opt_refs: Vec<&str> = options.iter().map(|s| s.as_str()).collect();
            let current_kernel = config.primary_kernel();
            let current_idx = archinstall_zfs_core::kernel::AVAILABLE_KERNELS
                .iter()
                .position(|k| k.name == current_kernel)
                .unwrap_or(0);
            show_select(
                app,
                "kernel_select",
                "Kernel",
                &opt_refs,
                current_idx as i32,
            );
        }
        "profile" => {
            let profiles = archinstall_zfs_core::profile::all_profiles();
            let mut names: Vec<String> = vec!["None".to_string()];
            names.extend(profiles.iter().map(|p| p.name.to_string()));
            let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
            let current = config
                .profile
                .as_ref()
                .and_then(|sel| profiles.iter().position(|p| p.name == *sel))
                .map(|i| (i + 1) as i32)
                .unwrap_or(0);
            show_select(app, "profile", "Profile", &refs, current);
        }
        "timezone" => {
            let regions = archinstall_zfs_core::installer::locale::list_timezone_regions();
            show_select(app, "timezone_region", "Timezone region", &regions, 0);
        }
        "locale" => {
            let locales = archinstall_zfs_core::installer::locale::list_locales();
            let locale_strs: Vec<&str> = locales.iter().map(|s| s.as_str()).collect();
            let current_idx = config
                .locale
                .as_ref()
                .and_then(|l| locales.iter().position(|x| x == l))
                .map(|i| i as i32)
                .unwrap_or(0);
            show_select_with_filter(
                app,
                "locale_select",
                "Locale (type to filter)",
                &locale_strs,
                current_idx,
                true,
            );
        }
        "users" => {
            show_users_popup(app);
        }
        "keyboard" => {
            let keymaps = archinstall_zfs_core::installer::locale::list_keymaps();
            let keymap_strs: Vec<&str> = keymaps.iter().map(|s| s.as_str()).collect();
            let current_idx = keymaps
                .iter()
                .position(|k| k == &config.keyboard_layout)
                .map(|i| i as i32)
                .unwrap_or(0);
            show_select_with_filter(
                app,
                "keyboard_select",
                "Keyboard layout (type to filter)",
                &keymap_strs,
                current_idx,
                true,
            );
        }
        "packages" => {
            show_package_search(app);
        }
        "extra_services" => {
            show_string_list(app, "Extra systemd services");
        }
        "pool_name"
        | "dataset_prefix"
        | "hostname"
        | "swap_partition_size"
        | "parallel_downloads" => {
            let (title, current) = match key {
                "pool_name" => ("Pool name", config.pool_name.clone().unwrap_or_default()),
                "dataset_prefix" => ("Dataset prefix", config.dataset_prefix.clone()),
                "hostname" => ("Hostname", config.hostname.clone().unwrap_or_default()),
                "swap_partition_size" => (
                    "Swap partition size",
                    config.swap_partition_size.clone().unwrap_or_default(),
                ),
                "parallel_downloads" => {
                    ("Parallel downloads", config.parallel_downloads.to_string())
                }
                _ => ("", String::new()),
            };
            show_text_input(app, key, title, &current, false);
        }
        "root_password" => {
            show_text_input(app, key, "Root password", "", true);
        }
        "encryption_password" => {
            show_text_input(app, key, "Encryption password", "", true);
        }
        _ => {}
    }
}

// ── Popup show helpers ──────────────────────────────

fn show_select(app: &App, key: &str, title: &str, options: &[&str], current: i32) {
    show_select_with_filter(app, key, title, options, current, false);
}

fn show_select_with_filter(
    app: &App,
    key: &str,
    title: &str,
    options: &[&str],
    current: i32,
    filterable: bool,
) {
    let opts: Vec<SelectOption> = options
        .iter()
        .map(|s| SelectOption {
            text: SharedString::from(*s),
        })
        .collect();
    let popup = app.global::<PopupState>();
    popup.set_select_key(key.into());
    popup.set_select_title(title.into());
    popup.set_select_options(ModelRc::new(VecModel::from(opts)));
    popup.set_select_index(current);
    popup.set_select_show_filter(filterable);
    popup.set_select_visible(true);
}

fn show_text_input(app: &App, key: &str, title: &str, current: &str, password: bool) {
    let popup = app.global::<PopupState>();
    popup.set_text_input_key(key.into());
    popup.set_text_input_title(title.into());
    popup.set_text_input_value(current.into());
    popup.set_text_input_password(password);
    popup.set_password_strength_score(-1);
    popup.set_password_strength_hint(SharedString::default());
    popup.set_text_input_visible(true);
}

fn show_users_popup(app: &App) {
    app.global::<PopupState>().set_users_visible(true);
}

fn show_string_list(app: &App, title: &str) {
    let popup = app.global::<PopupState>();
    popup.set_strlist_title(title.into());
    popup.set_strlist_visible(true);
}

fn show_package_search(app: &App) {
    let editing = app.global::<EditingState>();
    editing.set_package_searching_aur(false);
    editing.set_package_status_text(SharedString::default());
    app.global::<PopupState>().set_pkg_search_visible(true);
}
