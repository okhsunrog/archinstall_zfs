use color_eyre::eyre::Result;

use super::cli::run_zfs;
use crate::system::cmd::{CommandRunner, check_exit};
use palimpsest::dataset::{ListOptions, ZfsListEntry as PalimpsestZfsListEntry};

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

pub fn create_dataset(
    runner: &dyn CommandRunner,
    full_name: &str,
    properties: &[(&str, &str)],
) -> Result<()> {
    let mut args: Vec<&str> = vec!["create"];

    let owned: Vec<String> = properties
        .iter()
        .flat_map(|(k, v)| vec!["-o".to_string(), format!("{k}={v}")])
        .collect();
    let refs: Vec<&str> = owned.iter().map(|s| s.as_str()).collect();
    args.extend_from_slice(&refs);

    args.push(full_name);

    let output = run_zfs(runner, &args)?;
    check_exit(&output, &format!("zfs create {full_name}"))?;
    Ok(())
}

pub fn set_property(
    runner: &dyn CommandRunner,
    dataset: &str,
    property: &str,
    value: &str,
) -> Result<()> {
    let prop_val = format!("{property}={value}");
    let output = run_zfs(runner, &["set", &prop_val, dataset])?;
    check_exit(&output, &format!("zfs set {prop_val} {dataset}"))?;
    Ok(())
}

pub fn mount_dataset(runner: &dyn CommandRunner, dataset: &str) -> Result<()> {
    let output = run_zfs(runner, &["mount", dataset])?;
    check_exit(&output, &format!("zfs mount {dataset}"))?;
    Ok(())
}

pub fn umount_dataset(runner: &dyn CommandRunner, dataset: &str) -> Result<()> {
    let _ = run_zfs(runner, &["umount", dataset]);
    Ok(())
}

pub async fn list_datasets(
    runner: &dyn palimpsest::CommandRunner,
) -> Result<Vec<PalimpsestZfsListEntry>> {
    let opts = ListOptions::default();
    Ok(palimpsest::dataset::list(runner, &opts).await?)
}

/// Check if a dataset exists.
pub fn dataset_exists(runner: &dyn CommandRunner, name: &str) -> bool {
    run_zfs(runner, &["list", "-H", name]).is_ok_and(|output| output.success())
}

pub fn create_base_dataset(
    runner: &dyn CommandRunner,
    pool_name: &str,
    prefix: &str,
    encryption_props: &[(&str, &str)],
) -> Result<()> {
    let base_name = format!("{pool_name}/{prefix}");
    if dataset_exists(runner, &base_name) {
        color_eyre::eyre::bail!(
            "Dataset '{base_name}' already exists. \
             Choose a different dataset prefix or use Existing Pool mode."
        );
    }
    create_dataset(runner, &base_name, encryption_props)
}

pub fn create_child_datasets(
    runner: &dyn CommandRunner,
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
                create_dataset(runner, &parent_full, &[("mountpoint", "none")])?;
                created.insert(parent_full);
            }
        }

        let full_name = format!("{pool_name}/{prefix}/{}", ds.name);
        let props: Vec<(&str, &str)> = ds
            .properties
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        create_dataset(runner, &full_name, &props)?;
        created.insert(full_name);
    }
    Ok(())
}

pub fn mount_datasets_ordered(
    runner: &dyn CommandRunner,
    pool_name: &str,
    prefix: &str,
    _datasets: &[DatasetConfig],
) -> Result<()> {
    // Mount root dataset first (canmount=noauto)
    let root_ds = format!("{pool_name}/{prefix}/root");
    mount_dataset(runner, &root_ds)?;

    // Recursively mount all child datasets
    let base_ds = format!("{pool_name}/{prefix}");
    let output = run_zfs(runner, &["mount", "-R", &base_ds]);
    // -R may not be supported on all versions; fall back to mount -a
    if output.is_err() || !output.as_ref().unwrap().success() {
        let _ = run_zfs(runner, &["mount", "-a"]);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::cmd::tests::{CannedResponse, RecordingRunner};

    #[test]
    fn test_create_dataset_with_properties() {
        let runner = RecordingRunner::new(vec![CannedResponse::default()]);
        create_dataset(
            &runner,
            "pool/arch0/root",
            &[("mountpoint", "/"), ("canmount", "noauto")],
        )
        .unwrap();

        let calls = runner.calls();
        let args = &calls[0].args;
        assert!(args.contains(&"create".to_string()));
        assert!(args.contains(&"pool/arch0/root".to_string()));
        assert!(args.contains(&"mountpoint=/".to_string()));
        assert!(args.contains(&"canmount=noauto".to_string()));
    }

    #[test]
    fn test_create_child_datasets_sorts_by_depth() {
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

        // 3 datasets + 1 auto-created parent "data"
        let responses: Vec<CannedResponse> = (0..4).map(|_| CannedResponse::default()).collect();
        let runner = RecordingRunner::new(responses);
        create_child_datasets(&runner, "pool", "arch0", &datasets).unwrap();

        let calls = runner.calls();
        assert_eq!(calls.len(), 4);
        // "root" has 0 slashes, should come first
        assert!(calls[0].args.contains(&"pool/arch0/root".to_string()));
        // "data" auto-created as parent
        assert!(calls[1].args.contains(&"pool/arch0/data".to_string()));
        // Then data/home and data/root
    }

    #[test]
    fn test_default_datasets() {
        let ds = default_datasets();
        assert_eq!(ds.len(), 4);
        assert_eq!(ds[0].name, "root");
        assert_eq!(ds[1].name, "data/home");
        assert_eq!(ds[2].name, "data/root");
        assert_eq!(ds[3].name, "vm");
    }
}
