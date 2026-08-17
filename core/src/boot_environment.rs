//! Naming the datasets of a boot environment.
//!
//! A boot environment is a pool plus a prefix — `zroot` and `arch0` give
//! `zroot/arch0`, whose `root` child is what the system boots from. That
//! layout was spelled out with `format!` in six modules, each threading the
//! pool and the prefix through its own arguments, and each free to get the
//! order or the separator wrong on its own.
//!
//! One type carries the pair and answers what the datasets are called.

use std::fmt;

/// The datasets belonging to one boot environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootEnvironment {
    pool: String,
    prefix: String,
}

impl BootEnvironment {
    pub fn new(pool: impl Into<String>, prefix: impl Into<String>) -> Self {
        Self {
            pool: pool.into(),
            prefix: prefix.into(),
        }
    }

    /// The pool the environment lives in.
    pub fn pool(&self) -> &str {
        &self.pool
    }

    /// The name distinguishing this environment from others in the pool.
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// The environment's own dataset, parent of everything below.
    pub fn base(&self) -> String {
        format!("{}/{}", self.pool, self.prefix)
    }

    /// The dataset mounted at `/` when this environment is booted.
    pub fn root(&self) -> String {
        self.child("root")
    }

    /// A dataset within the environment, named relative to its base.
    pub fn child(&self, relative: &str) -> String {
        format!("{}/{relative}", self.base())
    }
}

impl fmt::Display for BootEnvironment {
    /// Prints the base dataset, which is how a boot environment is named
    /// everywhere it is shown or logged.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.base())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_built_from_the_pool_and_the_prefix() {
        let be = BootEnvironment::new("zroot", "arch0");

        assert_eq!(be.base(), "zroot/arch0");
        assert_eq!(be.root(), "zroot/arch0/root");
        assert_eq!(be.child("data/home"), "zroot/arch0/data/home");
        assert_eq!(be.to_string(), "zroot/arch0");
    }

    #[test]
    fn the_pool_and_prefix_stay_available_for_the_commands_that_need_them() {
        // zpool operations take the pool alone; the prefix appears in
        // messages about which environment is being built.
        let be = BootEnvironment::new("tank", "arch1");

        assert_eq!(be.pool(), "tank");
        assert_eq!(be.prefix(), "arch1");
    }

    #[test]
    fn environments_in_one_pool_are_distinct() {
        let first = BootEnvironment::new("zroot", "arch0");
        let second = BootEnvironment::new("zroot", "arch1");

        assert_ne!(first, second);
        assert_ne!(first.root(), second.root());
    }
}
