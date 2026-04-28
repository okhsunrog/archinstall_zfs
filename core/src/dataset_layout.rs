use color_eyre::eyre::{Result, bail};
use palimpsest::dataset::{CreateOptions, MountOptions};

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
    CreateOptions::new().properties(props.iter().map(|(k, v)| (k.to_string(), v.to_string())))
}

pub async fn create_dataset(
    zfs: &palimpsest::Zfs,
    full_name: &str,
    properties: &[(&str, &str)],
) -> Result<()> {
    zfs.create_dataset(full_name, &properties_to_opts(properties))
        .await?;
    Ok(())
}

/// Check if a dataset exists.
pub async fn dataset_exists(zfs: &palimpsest::Zfs, name: &str) -> bool {
    zfs.dataset(name).exists().await
}

pub async fn create_base_dataset(
    zfs: &palimpsest::Zfs,
    pool_name: &str,
    prefix: &str,
    encryption_props: &[(&str, &str)],
) -> Result<()> {
    let base_name = format!("{pool_name}/{prefix}");
    if dataset_exists(zfs, &base_name).await {
        bail!(
            "Dataset '{base_name}' already exists. \
             Choose a different dataset prefix or use Existing Pool mode."
        );
    }
    create_dataset(zfs, &base_name, encryption_props).await
}

pub async fn create_child_datasets(
    zfs: &palimpsest::Zfs,
    pool_name: &str,
    prefix: &str,
    datasets: &[DatasetConfig],
) -> Result<()> {
    // Sort by depth (number of slashes) to ensure parents are created first
    let mut sorted: Vec<&DatasetConfig> = datasets.iter().collect();
    sorted.sort_by_key(|d| d.name.matches('/').count());

    let mut created: std::collections::HashSet<String> = std::collections::HashSet::new();

    for ds in sorted {
        // Auto-create parent datasets if needed (e.g., "data" before "data/home")
        let parts: Vec<&str> = ds.name.split('/').collect();
        if parts.len() > 1 {
            let parent = parts[..parts.len() - 1].join("/");
            let parent_full = format!("{pool_name}/{prefix}/{parent}");
            if !created.contains(&parent_full) {
                create_dataset(zfs, &parent_full, &[("mountpoint", "none")]).await?;
                created.insert(parent_full);
            }
        }

        let full_name = format!("{pool_name}/{prefix}/{}", ds.name);
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
    zfs: &palimpsest::Zfs,
    pool_name: &str,
    prefix: &str,
    _datasets: &[DatasetConfig],
) -> Result<()> {
    // Mount root dataset first (canmount=noauto)
    let root_ds = format!("{pool_name}/{prefix}/root");
    zfs.dataset(&root_ds)
        .mount(&MountOptions::default())
        .await?;

    // Recursively mount all child datasets; fall back to mount -a if -R fails
    // (older or stripped-down ZFS builds occasionally lack -R).
    let base_ds = format!("{pool_name}/{prefix}");
    let recursive = MountOptions { recursive: true };
    if zfs.dataset(&base_ds).mount(&recursive).await.is_err() {
        let _ = zfs.mount_all().await;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use palimpsest::{Cmd, RecordingRunner, Zfs};

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
                Cmd::new("zfs").args(["create", "-o", "mountpoint=none", "pool/arch0/data"]),
                vec![],
                vec![],
                0,
            )
            .record(
                Cmd::new("zfs").args(["create", "-o", "mountpoint=/home", "pool/arch0/data/home"]),
                vec![],
                vec![],
                0,
            )
            .record(
                Cmd::new("zfs").args(["create", "-o", "mountpoint=/root", "pool/arch0/data/root"]),
                vec![],
                vec![],
                0,
            );

        let zfs = Zfs::with_runner(runner);
        create_child_datasets(&zfs, "pool", "arch0", &datasets)
            .await
            .expect("create_child_datasets succeeds");
    }
}
