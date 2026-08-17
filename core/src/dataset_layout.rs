use color_eyre::eyre::{Result, bail};

use crate::boot_environment::BootEnvironment;
use zfskit::dataset::{CreateOptions, MountOptions};

pub struct DatasetConfig {
    pub name: String,
    pub properties: Vec<(String, String)>,
}

pub fn default_datasets() -> Vec<DatasetConfig> {
    vec![
        DatasetConfig {
            name: "root".to_string(),
            properties: vec![
                ("mountpoint".to_string(), "/".to_string()),
                ("canmount".to_string(), "noauto".to_string()),
            ],
        },
        DatasetConfig {
            name: "data/home".to_string(),
            properties: vec![("mountpoint".to_string(), "/home".to_string())],
        },
        DatasetConfig {
            name: "data/root".to_string(),
            properties: vec![("mountpoint".to_string(), "/root".to_string())],
        },
        DatasetConfig {
            name: "vm".to_string(),
            properties: vec![("mountpoint".to_string(), "/vm".to_string())],
        },
    ]
}

fn properties_to_opts(props: &[(&str, &str)]) -> CreateOptions {
    CreateOptions::new()
        .no_mount()
        .properties(props.iter().map(|(k, v)| (k.to_string(), v.to_string())))
}

pub async fn create_dataset(
    zfs: &zfskit::Zfs,
    full_name: &str,
    properties: &[(&str, &str)],
) -> Result<()> {
    zfs.create_dataset(full_name, &properties_to_opts(properties))
        .await?;
    Ok(())
}

/// Check if a dataset exists.
pub async fn dataset_exists(zfs: &zfskit::Zfs, name: &str) -> Result<bool> {
    Ok(zfs.dataset(name)?.exists().await?)
}

pub async fn create_base_dataset(
    zfs: &zfskit::Zfs,
    be: &BootEnvironment,
    encryption_props: &[(&str, &str)],
) -> Result<()> {
    let base_name = be.base();
    if dataset_exists(zfs, &base_name).await? {
        bail!(
            "Dataset '{base_name}' already exists. \
             Choose a different dataset prefix or use Existing Pool mode."
        );
    }
    create_dataset(zfs, &base_name, encryption_props).await
}

pub async fn create_child_datasets(
    zfs: &zfskit::Zfs,
    be: &BootEnvironment,
    datasets: &[DatasetConfig],
) -> Result<()> {
    // Sort by depth (number of slashes) to ensure parents are created first
    let mut sorted: Vec<&DatasetConfig> = datasets.iter().collect();
    sorted.sort_by_key(|d| d.name.matches('/').count());

    let mut created: std::collections::HashSet<String> = std::collections::HashSet::new();

    for ds in sorted {
        // Auto-create the intermediate datasets a nested name implies, every
        // level of them: "a/b/c" needs both "a" and "a/b" to exist first, and
        // creating only the immediate parent leaves `zfs create` to fail on
        // the missing grandparent.
        let parts: Vec<&str> = ds.name.split('/').collect();
        for depth in 1..parts.len() {
            let ancestor = parts[..depth].join("/");
            let ancestor_full = be.child(&ancestor);
            if created.insert(ancestor_full.clone()) {
                create_dataset(zfs, &ancestor_full, &[("mountpoint", "none")]).await?;
            }
        }

        let full_name = be.child(&ds.name);
        let props: Vec<(&str, &str)> = ds
            .properties
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        create_dataset(zfs, &full_name, &props).await?;
        created.insert(full_name);
    }
    Ok(())
}

pub async fn mount_datasets_ordered(
    zfs: &zfskit::Zfs,
    be: &BootEnvironment,
    datasets: &[DatasetConfig],
) -> Result<()> {
    // Mount root dataset first (canmount=noauto)
    let root_ds = be.root();
    zfs.dataset(&root_ds)?
        .mount(&MountOptions::default())
        .await?;

    let mut children: Vec<&DatasetConfig> = datasets
        .iter()
        .filter(|dataset| dataset.name != "root")
        .collect();
    children.sort_by_key(|dataset| dataset.name.matches('/').count());

    for dataset in children {
        let full_name = be.child(&dataset.name);
        zfs.dataset(&full_name)?
            .mount(&MountOptions::default())
            .await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use zfskit::{Cmd, RecordingRunner, Zfs};

    #[test]
    fn test_default_datasets() {
        let ds = default_datasets();
        assert_eq!(ds.len(), 4);
        assert_eq!(ds[0].name, "root");
        assert_eq!(ds[1].name, "data/home");
        assert_eq!(ds[2].name, "data/root");
        assert_eq!(ds[3].name, "vm");
    }

    #[tokio::test]
    async fn test_create_child_datasets_sorts_by_depth_and_auto_parents() {
        // Three datasets: "data/home", "root", "data/root". After depth sort
        // we expect "root" first (0 slashes), then the auto-created "data"
        // parent, then "data/home" and "data/root". RecordingRunner keys on
        // the full Cmd, so we record exactly the four create calls we expect.
        let datasets = vec![
            DatasetConfig {
                name: "data/home".to_string(),
                properties: vec![("mountpoint".to_string(), "/home".to_string())],
            },
            DatasetConfig {
                name: "root".to_string(),
                properties: vec![
                    ("mountpoint".to_string(), "/".to_string()),
                    ("canmount".to_string(), "noauto".to_string()),
                ],
            },
            DatasetConfig {
                name: "data/root".to_string(),
                properties: vec![("mountpoint".to_string(), "/root".to_string())],
            },
        ];
        let runner = RecordingRunner::new()
            .record(
                Cmd::new("zfs").args([
                    "create",
                    "-u",
                    "-o",
                    "mountpoint=/",
                    "-o",
                    "canmount=noauto",
                    "pool/arch0/root",
                ]),
                vec![],
                vec![],
                0,
            )
            .record(
                Cmd::new("zfs").args(["create", "-u", "-o", "mountpoint=none", "pool/arch0/data"]),
                vec![],
                vec![],
                0,
            )
            .record(
                Cmd::new("zfs").args([
                    "create",
                    "-u",
                    "-o",
                    "mountpoint=/home",
                    "pool/arch0/data/home",
                ]),
                vec![],
                vec![],
                0,
            )
            .record(
                Cmd::new("zfs").args([
                    "create",
                    "-u",
                    "-o",
                    "mountpoint=/root",
                    "pool/arch0/data/root",
                ]),
                vec![],
                vec![],
                0,
            );

        let zfs = Zfs::with_runner(runner);
        create_child_datasets(&zfs, &BootEnvironment::new("pool", "arch0"), &datasets)
            .await
            .expect("create_child_datasets succeeds");
    }

    #[tokio::test]
    async fn nested_datasets_get_every_intermediate_level() {
        let datasets = vec![DatasetConfig {
            name: "data/media/photos".to_string(),
            properties: vec![("mountpoint".to_string(), "/photos".to_string())],
        }];
        let runner = RecordingRunner::new()
            .record(
                Cmd::new("zfs").args(["create", "-u", "-o", "mountpoint=none", "pool/arch0/data"]),
                vec![],
                vec![],
                0,
            )
            .record(
                Cmd::new("zfs").args([
                    "create",
                    "-u",
                    "-o",
                    "mountpoint=none",
                    "pool/arch0/data/media",
                ]),
                vec![],
                vec![],
                0,
            )
            .record(
                Cmd::new("zfs").args([
                    "create",
                    "-u",
                    "-o",
                    "mountpoint=/photos",
                    "pool/arch0/data/media/photos",
                ]),
                vec![],
                vec![],
                0,
            );

        let zfs = Zfs::with_runner(runner);
        create_child_datasets(&zfs, &BootEnvironment::new("pool", "arch0"), &datasets)
            .await
            .expect("every intermediate dataset is created");
    }

    #[tokio::test]
    async fn test_mount_datasets_mounts_only_selected_be() {
        let datasets = default_datasets();
        let runner = RecordingRunner::new()
            .record(
                Cmd::new("zfs").args(["mount", "pool/arch0/root"]),
                vec![],
                vec![],
                0,
            )
            .record(
                Cmd::new("zfs").args(["mount", "pool/arch0/vm"]),
                vec![],
                vec![],
                0,
            )
            .record(
                Cmd::new("zfs").args(["mount", "pool/arch0/data/home"]),
                vec![],
                vec![],
                0,
            )
            .record(
                Cmd::new("zfs").args(["mount", "pool/arch0/data/root"]),
                vec![],
                vec![],
                0,
            );

        let zfs = Zfs::with_runner(runner);
        mount_datasets_ordered(&zfs, &BootEnvironment::new("pool", "arch0"), &datasets)
            .await
            .expect("selected boot environment mounts");
    }

    #[tokio::test]
    async fn test_mount_datasets_propagates_nonempty_mountpoint_error() {
        let datasets = vec![
            DatasetConfig {
                name: "root".to_string(),
                properties: vec![
                    ("mountpoint".to_string(), "/".to_string()),
                    ("canmount".to_string(), "noauto".to_string()),
                ],
            },
            DatasetConfig {
                name: "data/root".to_string(),
                properties: vec![("mountpoint".to_string(), "/root".to_string())],
            },
        ];
        let runner = RecordingRunner::new()
            .record(
                Cmd::new("zfs").args(["mount", "pool/arch0/root"]),
                vec![],
                vec![],
                0,
            )
            .record(
                Cmd::new("zfs").args(["mount", "pool/arch0/data/root"]),
                vec![],
                b"cannot mount '/root': directory is not empty\n".to_vec(),
                1,
            );

        let zfs = Zfs::with_runner(runner);
        let error = mount_datasets_ordered(&zfs, &BootEnvironment::new("pool", "arch0"), &datasets)
            .await
            .expect_err("non-empty mountpoint must stop installation");
        assert!(error.to_string().contains("directory is not empty"));
    }
}
