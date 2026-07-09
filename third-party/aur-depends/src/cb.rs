use std::{any, fmt};

use alpm::Db;
use raur::Raur;

use crate::Resolver;

pub(crate) struct Callback<F: ?Sized>(pub(crate) Option<Box<F>>);

pub(crate) type ProviderCB = Callback<dyn Fn(&str, &[&str]) -> usize + 'static>;
pub(crate) type GroupCB<'a> = Callback<dyn Fn(&[Group<'a>]) -> Vec<&'a alpm::Package> + 'static>;
pub(crate) type IsDevelCb = Callback<dyn Fn(&str) -> bool + 'static>;

/// An alpm Db+Group pair passed to the group callback.
pub struct Group<'a> {
    /// The db the group belongs to.
    pub db: &'a Db,
    /// The group.
    pub group: &'a alpm::Group,
}

impl<T: ?Sized> fmt::Debug for Callback<T> {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.write_str(any::type_name::<Self>())
    }
}

impl<T: ?Sized> Default for Callback<T> {
    fn default() -> Self {
        Callback(None)
    }
}

impl<T: ?Sized> Callback<T> {
    pub fn get(&self) -> Option<&T> {
        self.0.as_deref()
    }
}

impl<'a, 'b, E: std::error::Error + Sync + Send + 'static, H: Raur<Err = E> + Sync>
    Resolver<'a, 'b, H>
{
    /// Set the provider callback
    ///
    /// The provider callback will be called any time there is a choice of multiple AUR packages
    /// that can satisfy a dependency. This callback receives the dependency that we are trying to
    /// satisfy and a slice of package names satisfying it.
    ///
    /// The callback returns returns the index of which package to pick.
    ///
    /// Retuning an invalid index will cause a panic.
    pub fn provider_callback<F: Fn(&str, &[&str]) -> usize + 'static>(mut self, f: F) -> Self {
        self.provider_callback = Callback(Some(Box::new(f)));
        self
    }

    /// Set the group callback
    ///
    /// The group callback is called whenever a group is processed. The callback recieves the group
    /// and returns a list of packages should be installed from the group;
    ///
    pub fn group_callback<F: Fn(&[Group<'a>]) -> Vec<&'a alpm::Package> + 'static>(
        mut self,
        f: F,
    ) -> Self {
        self.group_callback = Callback(Some(Box::new(f)));
        self
    }

    /// Set the function for determining if a package is devel.
    ///
    /// Devel packages are never skipped when using NEEDED.
    ///
    /// By default, no packages are considered devel.
    pub fn is_devel<F: Fn(&str) -> bool + 'static>(mut self, f: F) -> Self {
        self.is_devel = Callback(Some(Box::new(f)));
        self
    }
}
