//! Safe LinuxKMS demo session: read-only ZFS inventory and explicitly
//! read-only imports. The session owns only pools it imported itself and
//! exports those pools on request and on every process exit path.

use std::cell::RefCell;
use std::collections::HashSet;
use std::process::Command;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use color_eyre::eyre::{Result, bail};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use zfskit::dataset::ListOptions as DatasetListOptions;
use zfskit::pool::ListOptions as PoolListOptions;

use archinstall_zfs_core::config::types::GlobalConfig;

use crate::refresh::refresh_items;
use crate::ui::{App, DemoDataset, DemoPool, DemoState};

#[derive(Default)]
pub struct DemoSession {
    owned_pools: Mutex<HashSet<String>>,
}

impl DemoSession {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    async fn inventory(&self) -> Result<(Vec<DemoPool>, Vec<DemoDataset>)> {
        let zfs = zfskit::Zfs::new();
        let imported = zfs.list_pools(&PoolListOptions::default()).await?;
        let discovered = zfs.discover_importable_pools().await?;
        let owned = self.owned_pools.lock().unwrap().clone();

        let mut seen = HashSet::new();
        let mut pools = Vec::new();
        for pool in imported {
            seen.insert(pool.name.clone());
            pools.push(DemoPool {
                owned: owned.contains(&pool.name),
                name: pool.name.into(),
                state: pool.state.into(),
                imported: true,
            });
        }
        for pool in discovered {
            if seen.insert(pool.name.clone()) {
                pools.push(DemoPool {
                    name: pool.name.into(),
                    state: pool.state.into(),
                    imported: false,
                    owned: false,
                });
            }
        }
        pools.sort_by(|a, b| a.name.as_str().cmp(b.name.as_str()));

        let datasets = zfs
            .list_datasets(&DatasetListOptions {
                recursive: true,
                ..Default::default()
            })
            .await?
            .into_iter()
            .map(|dataset| DemoDataset {
                name: dataset.name.into(),
                kind: format!("{:?}", dataset.kind).to_lowercase().into(),
            })
            .collect();

        Ok((pools, datasets))
    }

    async fn import_readonly(&self, pool_name: &str) -> Result<()> {
        // Parse and validate before handing the name to a subprocess. No shell
        // is involved, but accepting only a valid ZFS pool name also keeps the
        // owned-pool cleanup set trustworthy.
        let zfs = zfskit::Zfs::new();
        let _ = zfs.pool(pool_name)?;
        let name = pool_name.to_string();
        let output = tokio::task::spawn_blocking(move || {
            Command::new("zpool")
                .args(readonly_import_args(&name))
                .output()
        })
        .await??;
        if !output.status.success() {
            bail!(
                "read-only import failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        self.owned_pools
            .lock()
            .unwrap()
            .insert(pool_name.to_string());

        let name = pool_name.to_string();
        let verification = tokio::task::spawn_blocking(move || {
            Command::new("zpool")
                .args(["get", "-H", "-o", "value", "readonly", &name])
                .output()
        })
        .await??;
        if !verification.status.success()
            || String::from_utf8_lossy(&verification.stdout).trim() != "on"
        {
            let _ = self.export_owned(pool_name).await;
            bail!("pool did not report readonly=on after import");
        }
        Ok(())
    }

    async fn export_owned(&self, pool_name: &str) -> Result<()> {
        let is_owned = self.owned_pools.lock().unwrap().contains(pool_name);
        if !is_owned {
            bail!("refusing to export a pool not imported by this demo session");
        }
        zfskit::Zfs::new()
            .pool(pool_name)?
            .export(&zfskit::pool::ExportOptions::default())
            .await?;
        self.owned_pools.lock().unwrap().remove(pool_name);
        Ok(())
    }

    pub fn export_all_blocking(&self) {
        let pools: Vec<String> = self.owned_pools.lock().unwrap().iter().cloned().collect();
        for pool in pools {
            let output = Command::new("zpool").args(["export", &pool]).output();
            match output {
                Ok(output) if output.status.success() => {
                    self.owned_pools.lock().unwrap().remove(&pool);
                }
                Ok(output) => tracing::error!(
                    pool,
                    stderr = %String::from_utf8_lossy(&output.stderr).trim(),
                    "failed to export demo-owned pool"
                ),
                Err(error) => tracing::error!(pool, %error, "failed to run zpool export"),
            }
        }
    }
}

fn readonly_import_args(pool_name: &str) -> [&str; 7] {
    [
        "import",
        "-N",
        "-o",
        "readonly=on",
        "-o",
        "cachefile=none",
        pool_name,
    ]
}

impl Drop for DemoSession {
    fn drop(&mut self) {
        self.export_all_blocking();
    }
}

pub fn setup(app: &App, config: &Rc<RefCell<GlobalConfig>>, session: &Arc<DemoSession>) {
    app.global::<DemoState>().set_enabled(true);

    let weak = app.as_weak();
    let session_for_refresh = session.clone();
    app.global::<DemoState>().on_refresh(move || {
        refresh_async(&weak, session_for_refresh.clone(), None);
    });

    let weak = app.as_weak();
    let session_for_import = session.clone();
    app.global::<DemoState>()
        .on_import_readonly(move |pool_name| {
            refresh_async(
                &weak,
                session_for_import.clone(),
                Some(DemoAction::Import(pool_name.to_string())),
            );
        });

    let weak = app.as_weak();
    let session_for_export = session.clone();
    app.global::<DemoState>().on_export_pool(move |pool_name| {
        refresh_async(
            &weak,
            session_for_export.clone(),
            Some(DemoAction::Export(pool_name.to_string())),
        );
    });

    let weak = app.as_weak();
    let cfg = config.clone();
    app.global::<DemoState>().on_select_pool(move |pool_name| {
        let Some(app) = weak.upgrade() else { return };
        cfg.borrow_mut().pool_name = Some(pool_name.to_string());
        refresh_items(&app, &cfg.borrow());
        app.global::<DemoState>()
            .set_status(SharedString::from(format!(
                "Selected pool {pool_name} for configuration"
            )));
    });
}

enum DemoAction {
    Import(String),
    Export(String),
}

fn refresh_async(weak: &slint::Weak<App>, session: Arc<DemoSession>, action: Option<DemoAction>) {
    let weak = weak.clone();
    if let Some(app) = weak.upgrade() {
        app.global::<DemoState>().set_busy(true);
        app.global::<DemoState>()
            .set_status("Refreshing ZFS inventory...".into());
    }
    tokio::spawn(async move {
        let action_result = match action {
            Some(DemoAction::Import(pool)) => session.import_readonly(&pool).await,
            Some(DemoAction::Export(pool)) => session.export_owned(&pool).await,
            None => Ok(()),
        };
        let inventory = match action_result {
            Ok(()) => session.inventory().await,
            Err(error) => Err(error),
        };
        let _ = weak.upgrade_in_event_loop(move |app| {
            let state = app.global::<DemoState>();
            state.set_busy(false);
            match inventory {
                Ok((pools, datasets)) => {
                    let pool_count = pools.len();
                    let dataset_count = datasets.len();
                    state.set_pools(ModelRc::new(VecModel::from(pools)));
                    state.set_datasets(ModelRc::new(VecModel::from(datasets)));
                    state.set_status(
                        format!("{pool_count} pool(s), {dataset_count} dataset(s)").into(),
                    );
                }
                Err(error) => state.set_status(format!("ZFS inventory failed: {error}").into()),
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::readonly_import_args;

    #[test]
    fn readonly_import_arguments_are_explicit() {
        assert_eq!(
            readonly_import_args("tank"),
            [
                "import",
                "-N",
                "-o",
                "readonly=on",
                "-o",
                "cachefile=none",
                "tank",
            ]
        );
    }
}
