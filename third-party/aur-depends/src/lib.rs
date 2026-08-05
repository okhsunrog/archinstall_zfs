#![warn(missing_docs)]

//! # aur-depend
//!
//! aur-depends is a dependency solving library for the AUR
//!
//! See [`Resolver`](struct.Resolver.html) for more info.

mod actions;
mod base;
mod cb;
mod error;
mod pkgbuild;
mod resolve;
mod satisfies;
#[cfg(test)]
mod tests;
mod upgrade;

pub use crate::actions::*;
pub use crate::base::*;
pub use crate::error::*;
pub use crate::pkgbuild::*;
pub use crate::resolve::*;
pub use crate::upgrade::*;
