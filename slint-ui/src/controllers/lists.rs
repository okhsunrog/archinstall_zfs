//! Controllers for the editable list popups: users, extra services, and
//! package search/selection. All edits update the canonical `GlobalConfig`
//! AND the matching `EditingModels` `VecModel<T>` in place, then trigger a
//! wizard items rebuild.

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle, Model, SharedString};

use archinstall_zfs_core::config::types::{GlobalConfig, UserConfig};

use crate::editing_models::{EditingModels, set_search_results};
use crate::refresh::refresh_items;
use crate::ui::{App, EditingState, PackageEntry, PackageSearchResult, SelectOption, UserEntry};

pub fn setup(app: &App, config: &Rc<RefCell<GlobalConfig>>, models: &EditingModels) {
    setup_users(app, config, models);
    setup_extra_services(app, config, models);
    setup_packages(app, config, models);
}

fn setup_users(app: &App, config: &Rc<RefCell<GlobalConfig>>, models: &EditingModels) {
    let weak = app.as_weak();
    let cfg = config.clone();
    let model = models.users.clone();
    app.on_user_added(move |username, password, sudo| {
        let Some(app) = weak.upgrade() else { return };
        let username = username.to_string();
        if !archinstall_zfs_core::config::validation::is_valid_username(&username) {
            return;
        }
        let mut c = cfg.borrow_mut();
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

    let weak = app.as_weak();
    let cfg = config.clone();
    let model = models.users.clone();
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

    let weak = app.as_weak();
    let cfg = config.clone();
    let model = models.users.clone();
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

fn setup_extra_services(app: &App, config: &Rc<RefCell<GlobalConfig>>, models: &EditingModels) {
    let weak = app.as_weak();
    let cfg = config.clone();
    let model = models.extra_services.clone();
    app.on_strlist_added(move |val| {
        let Some(app) = weak.upgrade() else { return };
        let val = val.to_string();
        if val.is_empty() {
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

    let weak = app.as_weak();
    let cfg = config.clone();
    let model = models.extra_services.clone();
    app.on_strlist_removed(move |index| {
        let Some(app) = weak.upgrade() else { return };
        let mut c = cfg.borrow_mut();
        let idx = index as usize;
        if idx < c.extra_services.len() {
            c.extra_services.remove(idx);
            model.remove(idx);
        }
        refresh_items(&app, &c);
    });
}

fn setup_packages(app: &App, config: &Rc<RefCell<GlobalConfig>>, models: &EditingModels) {
    // Repo search (alpm — runs blocking inside a tokio task)
    let weak = app.as_weak();
    let search_model = models.package_search.clone();
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

    // AUR search
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

    // Add a package by clicking a search result
    let weak = app.as_weak();
    let cfg = config.clone();
    let search_model = models.package_search.clone();
    let selected_model = models.packages_selected.clone();
    app.on_pkg_added(move |index| {
        let Some(app) = weak.upgrade() else { return };
        let Some(pkg) = search_model.row_data(index as usize) else {
            return;
        };
        let name = pkg.name.to_string();
        let mut c = cfg.borrow_mut();
        if c.additional_packages.contains(&name) || c.aur_packages.contains(&name) {
            return;
        }
        let entry = PackageEntry {
            name: SharedString::from(&name),
            repo: pkg.repo.clone(),
        };
        // Selected list renders repo-first then AUR — insert at the boundary
        // for repo additions, append for AUR.
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

    // Remove a package via the selected-list delete button
    let weak = app.as_weak();
    let cfg = config.clone();
    let selected_model = models.packages_selected.clone();
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
