//! ZFSBootMenu built from its release source, for distributions that neither
//! package it nor have the AUR to build it from.
//!
//! The release is pinned by version and checksum rather than taken from
//! whatever is newest: the image it produces is what the machine boots
//! through, so a new release arrives with an installer update that was
//! tested against it. The checksum is the one Arch's AUR package verifies.

use std::path::Path;
use std::sync::Arc;

use color_eyre::eyre::{Context, Result, bail};
use sha2::{Digest, Sha512};
use tokio_util::sync::CancellationToken;

use crate::system::cmd::{CommandRunner, chroot_checked};

pub struct Release {
    pub version: &'static str,
    /// SHA-512 of the GitHub source archive, lowercase hex.
    pub sha512: &'static str,
}

pub const RELEASE: Release = Release {
    version: "3.1.0",
    sha512: "79d2134e827e27dbcc170f47b7c8f465f64d70a1bb6ddb2f05553a0515ccdc75\
             e857e5378fd4fb9d61f3b07d3b0be8e163bde755dd09e0f7e599ebf14c575810",
};

/// Where the source is unpacked in the target, and kept: `make install` from
/// it is how a later release replaces this one.
const SOURCE_DIR: &str = "/usr/local/src/zfsbootmenu";

impl Release {
    fn url(&self) -> String {
        format!(
            "https://github.com/zbm-dev/zfsbootmenu/archive/v{}.tar.gz",
            self.version
        )
    }

    fn archive_name(&self) -> String {
        format!("zfsbootmenu-{}.tar.gz", self.version)
    }

    /// Refuse anything but the archive this release names.
    fn verify(&self, data: &[u8]) -> Result<()> {
        let digest = hex::encode(Sha512::digest(data));
        if digest != self.sha512 {
            bail!(
                "ZFSBootMenu {} archive has SHA-512 {digest}, expected {}",
                self.version,
                self.sha512
            );
        }
        Ok(())
    }
}

/// Download, verify and install the release into `target`: `generate-zbm`,
/// its library and the dracut module it builds images with. The build
/// dependencies must already be installed there.
pub async fn install(
    runner: Arc<dyn CommandRunner>,
    target: &Path,
    release: &Release,
    cancel: &CancellationToken,
) -> Result<()> {
    let url = release.url();
    tracing::info!(
        version = release.version,
        url,
        "downloading ZFSBootMenu source"
    );
    let client = reqwest::Client::builder()
        .user_agent("archinstall-zfs-rs")
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .wrap_err("failed to create HTTP client")?;
    let data = tokio::select! {
        _ = cancel.cancelled() => bail!("installation cancelled"),
        data = async {
            client.get(&url).send().await?.error_for_status()?.bytes().await
        } => data.wrap_err_with(|| format!("cannot download {url}"))?,
    };
    release.verify(&data)?;

    let archive = format!("/usr/local/src/{}", release.archive_name());
    let archive_path = target.join(archive.trim_start_matches('/'));
    if let Some(parent) = archive_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&archive_path, &data)
        .wrap_err_with(|| format!("cannot write {}", archive_path.display()))?;

    let target = target.to_path_buf();
    tokio::task::spawn_blocking(move || build(&*runner, &target, &archive)).await?
}

/// Unpack over any earlier release and install what the dracut image build
/// needs; the mkinitcpio parts are left out.
fn build(runner: &dyn CommandRunner, target: &Path, archive: &str) -> Result<()> {
    let script = format!(
        "set -e; rm -rf {SOURCE_DIR}; mkdir -p {SOURCE_DIR}; \
         tar -xzf {archive} -C {SOURCE_DIR} --strip-components=1; \
         make -C {SOURCE_DIR} core dracut"
    );
    chroot_checked(
        runner,
        target,
        "sh",
        &["-c", &script],
        "build ZFSBootMenu from source",
    )?;
    tracing::info!("ZFSBootMenu installed from source");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::cmd::tests::{CannedResponse, RecordingRunner};

    #[test]
    fn the_pinned_checksum_is_a_sha512() {
        assert_eq!(RELEASE.sha512.len(), 128);
        assert!(
            RELEASE
                .sha512
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
        assert!(RELEASE.url().ends_with("/archive/v3.1.0.tar.gz"));
    }

    #[test]
    fn only_the_named_archive_is_accepted() {
        let release = Release {
            version: "0",
            sha512: "ee26b0dd4af7e749aa1a8ee3c10ae9923f618980772e473f8819a5d4940e0db2\
                     7ac185f8a0e1d5f84f88bc887fd67b143732c304cc5fa9ad8e6f57f50028a8ff",
        };

        release.verify(b"test").unwrap();
        let error = release.verify(b"tampered").unwrap_err().to_string();
        assert!(error.contains("expected ee26b0dd"), "{error}");
    }

    #[test]
    fn the_build_installs_the_core_and_the_dracut_module() {
        let dir = tempfile::tempdir().unwrap();
        let runner = RecordingRunner::new(vec![CannedResponse::default()]);

        build(
            &runner,
            dir.path(),
            "/usr/local/src/zfsbootmenu-3.1.0.tar.gz",
        )
        .unwrap();

        let calls = runner.calls();
        assert_eq!(calls[0].program, "arch-chroot");
        let script = calls[0].args.last().unwrap();
        assert!(script.starts_with("set -e;"), "{script}");
        assert!(script.contains("--strip-components=1"), "{script}");
        assert!(
            script.ends_with("make -C /usr/local/src/zfsbootmenu core dracut"),
            "{script}"
        );
    }

    #[test]
    fn a_failed_build_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let runner = RecordingRunner::new(vec![CannedResponse {
            exit_code: 2,
            stderr: "make: *** No rule to make target 'dracut'".into(),
            ..Default::default()
        }]);

        let error = build(&runner, dir.path(), "/x.tar.gz")
            .unwrap_err()
            .to_string();
        assert!(error.contains("build ZFSBootMenu"), "{error}");
    }
}
