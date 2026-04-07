//! Long-lived `VecModel<T>`s for editable lists, mutated incrementally by
//! the list/users/packages controllers and read by the popup components.

use std::rc::Rc;

use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use archinstall_zfs_core::config::types::GlobalConfig;

use crate::ui::{
    App, EditingState, MultiSelectOption, PackageEntry, PackageSearchResult, SelectOption,
    UserEntry,
};

#[derive(Clone)]
pub struct EditingModels {
    pub users: Rc<VecModel<UserEntry>>,
    pub extra_services: Rc<VecModel<SelectOption>>,
    pub packages_selected: Rc<VecModel<PackageEntry>>,
    pub package_search: Rc<VecModel<PackageSearchResult>>,
    /// Backing model for `MultiSelectPopup`. Reseeded each time the popup
    /// opens; rows are mutated in place when the user toggles them.
    pub multi_select: Rc<VecModel<MultiSelectOption>>,
}

impl EditingModels {
    pub fn new() -> Self {
        Self {
            users: Rc::new(VecModel::default()),
            extra_services: Rc::new(VecModel::default()),
            packages_selected: Rc::new(VecModel::default()),
            package_search: Rc::new(VecModel::default()),
            multi_select: Rc::new(VecModel::default()),
        }
    }

    /// Wire the editing `VecModel`s into the EditingState global once at startup.
    pub fn attach(&self, app: &App) {
        let editing = app.global::<EditingState>();
        editing.set_users(self.users.clone().into());
        editing.set_extra_services(self.extra_services.clone().into());
        editing.set_packages_selected(self.packages_selected.clone().into());
        editing.set_package_search_results(self.package_search.clone().into());
        editing.set_multi_select_options(self.multi_select.clone().into());
    }

    /// Mirror the canonical config lists into the live VecModels.
    pub fn seed(&self, config: &GlobalConfig) {
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
        self.users.set_vec(users);

        let services: Vec<SelectOption> = config
            .extra_services
            .iter()
            .map(|s| SelectOption {
                text: SharedString::from(s.as_str()),
            })
            .collect();
        self.extra_services.set_vec(services);

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
        self.packages_selected.set_vec(packages);
    }
}

/// Replace the contents of the package-search-results VecModel held inside
/// EditingState. Async tasks call this so they don't have to capture a non-Send
/// `Rc<VecModel<_>>`.
pub fn set_search_results(app: &App, items: Vec<PackageSearchResult>) {
    let model: ModelRc<PackageSearchResult> =
        app.global::<EditingState>().get_package_search_results();
    let vec_model = model
        .as_any()
        .downcast_ref::<VecModel<PackageSearchResult>>()
        .expect("package_search_results is always a VecModel");
    vec_model.set_vec(items);
}

/// Replace the contents of the multi-select VecModel. Used when opening the
/// MultiSelectPopup with a fresh set of options.
pub fn set_multi_select_options(app: &App, items: Vec<crate::ui::MultiSelectOption>) {
    let model: ModelRc<crate::ui::MultiSelectOption> =
        app.global::<EditingState>().get_multi_select_options();
    let vec_model = model
        .as_any()
        .downcast_ref::<VecModel<crate::ui::MultiSelectOption>>()
        .expect("multi_select_options is always a VecModel");
    vec_model.set_vec(items);
}
