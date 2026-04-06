mod config_items;
mod controllers;
mod format;
mod install;
mod tracing_layer;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use color_eyre::eyre::{Result, bail};
use slint::{Model, ModelRc, SharedString, VecModel};

use archinstall_zfs_core::config::types::{GlobalConfig, UserConfig};

pub mod ui {
    slint::include_modules!();
}
use ui::*;

use config_items::{apply_radio, apply_text, build_step_items, next_selectable_index};

#[derive(Parser, Debug)]
#[command(
    name = "archinstall-zfs",
    about = "Arch Linux installer with ZFS support (Slint UI)"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    #[arg(long, global = true)]
    config: Option<PathBuf>,

    #[arg(long, global = true)]
    silent: bool,

    /// UI scale factor for the GUI (e.g. 1.5, 2.0). On linuxkms this maps to
    /// the SLINT_SCALE_FACTOR env var since the backend cannot auto-detect
    /// physical DPI; on desktop builds the OS value is used unless overridden.
    #[arg(long, global = true)]
    ui_scale: Option<f32>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    RenderProfile {
        #[arg(long)]
        profile_dir: PathBuf,
        #[arg(long)]
        out_dir: PathBuf,
        #[arg(long, default_value = "linux-lts")]
        kernel: String,
        #[arg(long, default_value = "precompiled")]
        zfs: String,
        #[arg(long, default_value = "auto")]
        headers: String,
        #[arg(long)]
        fast: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let cli = Cli::parse();

    if let Some(scale) = cli.ui_scale
        && scale > 0.0
    {
        // Must be set before any Slint window is created.
        // SAFETY: single-threaded at this point in startup.
        unsafe {
            std::env::set_var("SLINT_SCALE_FACTOR", scale.to_string());
        }
    }

    match &cli.command {
        Some(Commands::RenderProfile {
            profile_dir,
            out_dir,
            kernel,
            zfs,
            headers,
            fast,
        }) => archinstall_zfs_core::iso::render_profile(
            profile_dir,
            out_dir,
            kernel,
            zfs,
            headers,
            *fast,
        ),
        None => {
            let config = if let Some(ref path) = cli.config {
                GlobalConfig::load_from_file(path)?
            } else {
                GlobalConfig::default()
            };

            if cli.silent {
                if cli.config.is_none() {
                    bail!("--silent requires --config");
                }
                let errors = config.validate_for_install();
                if !errors.is_empty() {
                    bail!("Config validation failed:\n  {}", errors.join("\n  "));
                }
                let runner: Arc<dyn archinstall_zfs_core::system::cmd::CommandRunner> =
                    Arc::new(archinstall_zfs_core::system::cmd::RealRunner);
                install::run_install(runner, &config, None)
            } else {
                run_gui(config)
            }
        }
    }
}

fn run_gui(config: GlobalConfig) -> Result<()> {
    let app = App::new()?;
    let config = Rc::new(RefCell::new(config));
    let kernel_scan: controllers::welcome::KernelScan = Arc::new(std::sync::Mutex::new(None));

    // ── Editable list models (long-lived, mutated incrementally) ──
    let users_model: Rc<VecModel<UserEntry>> = Rc::new(VecModel::default());
    let extra_services_model: Rc<VecModel<SelectOption>> = Rc::new(VecModel::default());
    let packages_selected_model: Rc<VecModel<PackageEntry>> = Rc::new(VecModel::default());
    let package_search_model: Rc<VecModel<PackageSearchResult>> = Rc::new(VecModel::default());

    {
        let editing = app.global::<EditingState>();
        editing.set_users(users_model.clone().into());
        editing.set_extra_services(extra_services_model.clone().into());
        editing.set_packages_selected(packages_selected_model.clone().into());
        editing.set_package_search_results(package_search_model.clone().into());
    }

    seed_editing_state(
        &config.borrow(),
        &users_model,
        &extra_services_model,
        &packages_selected_model,
    );

    refresh_items(&app, &config.borrow());

    controllers::welcome::setup(&app, &config, &kernel_scan);

    // ── Wizard step changed (rebuild items) ─────────
    {
        let weak = app.as_weak();
        let cfg = config.clone();
        app.global::<WizardState>().on_step_changed(move |_step| {
            let Some(app) = weak.upgrade() else { return };
            refresh_items(&app, &cfg.borrow());
        });
    }

    // ── Item activated ───────────────────────────────
    {
        let weak = app.as_weak();
        let cfg = config.clone();
        let kscan = kernel_scan.clone();
        app.on_item_activated(move |key| {
            let Some(app) = weak.upgrade() else { return };

            // Handle inline radio option clicks: "radio:{group_key}:{index}"
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

    // ── Toggle activated ─────────────────────────────
    {
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

    // ── Select confirmed ─────────────────────────────
    {
        let weak = app.as_weak();
        let cfg = config.clone();
        let kscan = kernel_scan.clone();
        app.on_select_confirmed(move |key, idx| {
            let Some(app) = weak.upgrade() else { return };

            if key == "timezone_region" {
                let regions = archinstall_zfs_core::installer::locale::list_timezone_regions();
                if let Some(&region) = regions.get(idx as usize) {
                    let cities =
                        archinstall_zfs_core::installer::locale::list_timezone_cities(region);
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
                    // Auto-set ZFS module mode from scan results
                    if let Some(ref cached) = *kscan.lock().unwrap()
                        && let Some(result) = cached.get(idx as usize)
                        && let Some(mode) = result.best_mode()
                    {
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

    // ── Text confirmed ───────────────────────────────
    {
        let weak = app.as_weak();
        let cfg = config.clone();
        app.on_text_confirmed(move |key, val| {
            let Some(app) = weak.upgrade() else { return };
            let mut c = cfg.borrow_mut();
            apply_text(&mut c, &key, &val);
            refresh_items(&app, &c);
        });
    }

    // ── User management ─────────────────────────────
    {
        let weak = app.as_weak();
        let cfg = config.clone();
        let model = users_model.clone();
        app.on_user_added(move |username, password, sudo| {
            let Some(app) = weak.upgrade() else { return };
            let username = username.to_string();
            if !archinstall_zfs_core::config::validation::is_valid_username(&username) {
                return;
            }
            let mut c = cfg.borrow_mut();
            // Prevent duplicate usernames
            if c.users
                .as_ref()
                .is_some_and(|users| users.iter().any(|u| u.username == username))
            {
                return;
            }
            let password = if password.is_empty() {
                None
            } else {
                Some(password.to_string())
            };
            let user = UserConfig {
                username: username.clone(),
                password,
                sudo,
                shell: None,
                groups: None,
                ssh_authorized_keys: Vec::new(),
                autologin: false,
            };
            c.users.get_or_insert_with(Vec::new).push(user);
            model.push(UserEntry {
                username: SharedString::from(&username),
                has_sudo: sudo,
            });
            refresh_items(&app, &c);
        });
    }
    {
        let weak = app.as_weak();
        let cfg = config.clone();
        let model = users_model.clone();
        app.on_user_removed(move |index| {
            let Some(app) = weak.upgrade() else { return };
            let mut c = cfg.borrow_mut();
            let idx = index as usize;
            if let Some(ref mut users) = c.users
                && idx < users.len()
            {
                users.remove(idx);
                if users.is_empty() {
                    c.users = None;
                }
                model.remove(idx);
            }
            refresh_items(&app, &c);
        });
    }
    {
        let weak = app.as_weak();
        let cfg = config.clone();
        let model = users_model.clone();
        app.on_user_sudo_toggled(move |index| {
            let Some(app) = weak.upgrade() else { return };
            let mut c = cfg.borrow_mut();
            let idx = index as usize;
            if let Some(ref mut users) = c.users
                && let Some(user) = users.get_mut(idx)
            {
                user.sudo = !user.sudo;
                if let Some(mut entry) = model.row_data(idx) {
                    entry.has_sudo = user.sudo;
                    model.set_row_data(idx, entry);
                }
            }
            refresh_items(&app, &c);
        });
    }

    // ── String list (extra_services etc.) ─────────────
    {
        let weak = app.as_weak();
        let cfg = config.clone();
        let model = extra_services_model.clone();
        app.on_strlist_added(move |key, val| {
            let Some(app) = weak.upgrade() else { return };
            let val = val.to_string();
            if val.is_empty() || key.as_str() != "extra_services" {
                return;
            }
            let mut c = cfg.borrow_mut();
            if !c.extra_services.contains(&val) {
                c.extra_services.push(val.clone());
                model.push(SelectOption {
                    text: SharedString::from(&val),
                });
            }
            refresh_items(&app, &c);
        });
    }
    {
        let weak = app.as_weak();
        let cfg = config.clone();
        let model = extra_services_model.clone();
        app.on_strlist_removed(move |key, index| {
            let Some(app) = weak.upgrade() else { return };
            if key.as_str() != "extra_services" {
                return;
            }
            let mut c = cfg.borrow_mut();
            let idx = index as usize;
            if idx < c.extra_services.len() {
                c.extra_services.remove(idx);
                model.remove(idx);
            }
            refresh_items(&app, &c);
        });
    }

    // ── Package search ───────────────────────────────
    {
        let weak = app.as_weak();
        let search_model = package_search_model.clone();
        app.on_pkg_search_changed(move |text| {
            let Some(app) = weak.upgrade() else { return };
            let editing = app.global::<EditingState>();
            if text.is_empty() {
                search_model.set_vec(Vec::<PackageSearchResult>::new());
                editing.set_package_status_text(SharedString::default());
                return;
            }
            editing.set_package_searching_aur(false);
            let query = text.to_string();
            let weak2 = app.as_weak();
            // Repo search is blocking (alpm) — run async
            tokio::spawn(async move {
                let results = archinstall_zfs_core::packages::search_repo(&query, 20)
                    .await
                    .unwrap_or_default();
                let items: Vec<PackageSearchResult> = results
                    .into_iter()
                    .map(|p| PackageSearchResult {
                        name: SharedString::from(&p.name),
                        description: SharedString::from(&p.description),
                        repo: SharedString::from(&p.repo),
                    })
                    .collect();
                let _ = weak2.upgrade_in_event_loop(move |app| {
                    set_search_results(&app, items);
                    app.global::<EditingState>()
                        .set_package_status_text(SharedString::default());
                });
            });
        });
    }
    {
        let weak = app.as_weak();
        app.on_pkg_search_aur(move |text| {
            let Some(app) = weak.upgrade() else { return };
            if text.is_empty() {
                return;
            }
            let editing = app.global::<EditingState>();
            editing.set_package_searching_aur(true);
            editing.set_package_status_text(SharedString::from("Searching AUR..."));
            let query = text.to_string();
            let weak2 = app.as_weak();
            tokio::spawn(async move {
                match archinstall_zfs_core::packages::search_aur(&query, 20).await {
                    Ok(results) => {
                        let items: Vec<PackageSearchResult> = results
                            .into_iter()
                            .map(|p| PackageSearchResult {
                                name: SharedString::from(&p.name),
                                description: SharedString::from(&p.description),
                                repo: SharedString::from(&p.repo),
                            })
                            .collect();
                        let _ = weak2.upgrade_in_event_loop(move |app| {
                            set_search_results(&app, items);
                            app.global::<EditingState>()
                                .set_package_status_text(SharedString::default());
                        });
                    }
                    Err(e) => {
                        let msg = format!("AUR error: {e}");
                        let _ = weak2.upgrade_in_event_loop(move |app| {
                            app.global::<EditingState>()
                                .set_package_status_text(SharedString::from(&msg));
                        });
                    }
                }
            });
        });
    }
    {
        let weak = app.as_weak();
        let cfg = config.clone();
        let search_model = package_search_model.clone();
        let selected_model = packages_selected_model.clone();
        app.on_pkg_added(move |index| {
            let Some(app) = weak.upgrade() else { return };
            let Some(pkg) = search_model.row_data(index as usize) else {
                return;
            };
            let name = pkg.name.to_string();
            let mut c = cfg.borrow_mut();
            // Check duplicates
            if c.additional_packages.contains(&name) || c.aur_packages.contains(&name) {
                return;
            }
            let entry = PackageEntry {
                name: SharedString::from(&name),
                repo: pkg.repo.clone(),
            };
            // Selected list is rendered repo-first, then AUR — insert at the
            // boundary for repo additions, append for AUR.
            if pkg.repo == "aur" {
                c.aur_packages.push(name);
                selected_model.push(entry);
            } else {
                let insert_at = c.additional_packages.len();
                c.additional_packages.push(name);
                selected_model.insert(insert_at, entry);
            }
            refresh_items(&app, &c);
        });
    }
    {
        let weak = app.as_weak();
        let cfg = config.clone();
        let selected_model = packages_selected_model.clone();
        app.on_pkg_removed(move |index| {
            let Some(app) = weak.upgrade() else { return };
            let mut c = cfg.borrow_mut();
            let idx = index as usize;
            let repo_len = c.additional_packages.len();
            if idx < repo_len {
                c.additional_packages.remove(idx);
                selected_model.remove(idx);
            } else {
                let aur_idx = idx - repo_len;
                if aur_idx < c.aur_packages.len() {
                    c.aur_packages.remove(aur_idx);
                    selected_model.remove(idx);
                }
            }
            refresh_items(&app, &c);
        });
    }

    controllers::install::setup(&app, &config);

    // ── Keyboard navigation ────────────────────────
    {
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
    }
    {
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
    }
    {
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

    // ── Select filter changed (fuzzy search for locale etc.) ──
    {
        let weak = app.as_weak();
        app.on_select_filter_changed(move |key, filter_text| {
            let Some(app) = weak.upgrade() else { return };
            let filter = filter_text.to_lowercase();

            if key == "locale_select" {
                let all_locales = archinstall_zfs_core::installer::locale::list_locales();
                let filtered: Vec<SelectOption> = if filter.is_empty() {
                    all_locales
                        .iter()
                        .map(|s| SelectOption {
                            text: SharedString::from(s.as_str()),
                        })
                        .collect()
                } else {
                    let mut scored: Vec<_> = all_locales
                        .iter()
                        .filter_map(|s| {
                            sublime_fuzzy::best_match(&filter, s).map(|m| (m.score(), s))
                        })
                        .collect();
                    scored.sort_by(|a, b| b.0.cmp(&a.0));
                    scored
                        .into_iter()
                        .map(|(_, s)| SelectOption {
                            text: SharedString::from(s.as_str()),
                        })
                        .collect()
                };
                app.global::<PopupState>()
                    .set_select_options(ModelRc::new(VecModel::from(filtered)));
                app.global::<PopupState>().set_select_index(-1);
            }

            if key == "keyboard_select" {
                let all_keymaps = archinstall_zfs_core::installer::locale::list_keymaps();
                let filtered: Vec<SelectOption> = if filter.is_empty() {
                    all_keymaps
                        .iter()
                        .map(|s| SelectOption {
                            text: SharedString::from(s.as_str()),
                        })
                        .collect()
                } else {
                    let mut scored: Vec<_> = all_keymaps
                        .iter()
                        .filter_map(|s| {
                            sublime_fuzzy::best_match(&filter, s).map(|m| (m.score(), s))
                        })
                        .collect();
                    scored.sort_by(|a, b| b.0.cmp(&a.0));
                    scored
                        .into_iter()
                        .map(|(_, s)| SelectOption {
                            text: SharedString::from(s.as_str()),
                        })
                        .collect()
                };
                app.global::<PopupState>()
                    .set_select_options(ModelRc::new(VecModel::from(filtered)));
                app.global::<PopupState>().set_select_index(-1);
            }
        });
    }

    // ── Text input edited (password strength) ───────
    {
        let weak = app.as_weak();
        app.on_text_input_edited(move |key, value| {
            let Some(app) = weak.upgrade() else { return };

            if key == "root_password" || key == "encryption_password" {
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

                app.global::<PopupState>()
                    .set_password_strength_score(score as i32);
                app.global::<PopupState>()
                    .set_password_strength_label(SharedString::from(label));
                app.global::<PopupState>()
                    .set_password_strength_hint(SharedString::from(hint));
                app.global::<PopupState>()
                    .set_password_strength_color(color);
            }
        });
    }

    // ── Quit ─────────────────────────────────────────
    {
        let weak = app.as_weak();
        app.on_quit_requested(move || {
            if let Some(app) = weak.upgrade() {
                let should_reboot = app.global::<InstallState>().get_state() == 2;
                let _ = app.window().hide();
                if should_reboot {
                    let _ = std::process::Command::new("systemctl")
                        .arg("reboot")
                        .spawn();
                }
            }
        });
    }

    app.run()?;
    Ok(())
}

// ── UI refresh ──────────────────────────────────────

fn refresh_items(app: &App, config: &GlobalConfig) {
    let step = app.global::<WizardState>().get_current_step() as usize;
    let items = build_step_items(step, config);
    let first = next_selectable_index(&items, -1, 1);
    app.global::<WizardState>().set_focused_index(first);
    app.global::<WizardState>()
        .set_config_items(ModelRc::new(VecModel::from(items)));
    app.global::<WizardState>()
        .set_status_text(SharedString::default());
}

// ── Item activation (open popup) ─────────────────────

fn handle_item_activated(
    app: &App,
    key: &str,
    config: &GlobalConfig,
    kernel_scan: &Arc<
        std::sync::Mutex<Option<Vec<archinstall_zfs_core::kernel::scanner::CompatibilityResult>>>,
    >,
) {
    match key {
        // Popup selects — only for items with too many options or async scan
        "kernel" => {
            let cached = kernel_scan.lock().unwrap();
            let results: Vec<archinstall_zfs_core::kernel::scanner::CompatibilityResult>;
            let scan = if let Some(ref cached_results) = *cached {
                cached_results
            } else {
                drop(cached);
                let rt = tokio::runtime::Handle::current();
                results = rt.block_on(archinstall_zfs_core::kernel::scanner::scan_all_kernels());
                &results
            };

            // Only show compatible kernels (best_mode is Some)
            let mut options = Vec::new();
            for (info, result) in archinstall_zfs_core::kernel::AVAILABLE_KERNELS
                .iter()
                .zip(scan.iter())
            {
                let ver = result.kernel_version.as_deref().unwrap_or("?");
                if result.best_mode().is_some() {
                    options.push(format!(
                        "\u{2713} {} ({}) [{}]",
                        info.display_name,
                        ver,
                        result.mode_label()
                    ));
                } else {
                    options.push(format!(
                        "\u{2717} {} ({}) [incompatible]",
                        info.display_name, ver
                    ));
                }
            }
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
        // User management popup
        "users" => {
            show_users_popup(app);
        }
        // Keyboard layout (filterable select)
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
        // Text input popups
        // Package search popup
        "packages" => {
            show_package_search(app);
        }
        // String list popup
        "extra_services" => {
            show_string_list(app, key, "Extra systemd services");
        }
        // Text input popups
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
    app.set_users_visible(true);
}

/// Replace the contents of the package-search-results VecModel held inside
/// EditingState. We pull the model out of the global rather than capturing it,
/// so async tasks don't need to hold a non-Send `Rc<VecModel<_>>`.
fn set_search_results(app: &App, items: Vec<PackageSearchResult>) {
    let model = app.global::<EditingState>().get_package_search_results();
    let vec_model = model
        .as_any()
        .downcast_ref::<VecModel<PackageSearchResult>>()
        .expect("package_search_results is always a VecModel");
    vec_model.set_vec(items);
}

fn show_string_list(app: &App, key: &str, title: &str) {
    app.set_strlist_key(key.into());
    app.set_strlist_title(title.into());
    app.set_strlist_visible(true);
}

fn show_package_search(app: &App) {
    let editing = app.global::<EditingState>();
    editing.set_package_searching_aur(false);
    editing.set_package_status_text(SharedString::default());
    app.set_pkg_search_visible(true);
}

/// Mirror the canonical config lists into the long-lived Slint VecModels.
/// Called once at startup.
fn seed_editing_state(
    config: &GlobalConfig,
    users_model: &Rc<VecModel<UserEntry>>,
    extra_services_model: &Rc<VecModel<SelectOption>>,
    packages_selected_model: &Rc<VecModel<PackageEntry>>,
) {
    let users: Vec<UserEntry> = config
        .users
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|u| UserEntry {
            username: SharedString::from(&u.username),
            has_sudo: u.sudo,
        })
        .collect();
    users_model.set_vec(users);

    let services: Vec<SelectOption> = config
        .extra_services
        .iter()
        .map(|s| SelectOption {
            text: SharedString::from(s.as_str()),
        })
        .collect();
    extra_services_model.set_vec(services);

    let packages: Vec<PackageEntry> = config
        .additional_packages
        .iter()
        .map(|s| PackageEntry {
            name: SharedString::from(s.as_str()),
            repo: SharedString::from("repo"),
        })
        .chain(config.aur_packages.iter().map(|s| PackageEntry {
            name: SharedString::from(s.as_str()),
            repo: SharedString::from("aur"),
        }))
        .collect();
    packages_selected_model.set_vec(packages);
}
