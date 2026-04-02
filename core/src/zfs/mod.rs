pub mod bootmenu;
pub mod cache;
pub mod cli;
pub mod dataset;
pub mod encryption;
pub mod kmod;
pub mod models;
pub mod pool;

pub const ZFS_SERVICES: &[&str] = &[
    "zfs.target",
    "zfs-import.target",
    "zfs-volumes.target",
    "zfs-import-scan.service",
    "zfs-zed.service",
];
