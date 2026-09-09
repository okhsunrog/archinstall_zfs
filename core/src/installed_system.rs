//! Non-secret identity of a completed installation, captured before pool export.

use color_eyre::eyre::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::system::cmd::{CommandRunner, check_exit};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledSystem {
    pub pool_name: String,
    pub pool_guid: String,
    pub dataset_prefix: String,
    pub efi_partuuid: String,
}

impl InstalledSystem {
    pub fn capture(
        runner: &dyn CommandRunner,
        pool_name: &str,
        dataset_prefix: &str,
        efi: &Path,
    ) -> Result<Self> {
        let guid = runner.run("zpool", &["get", "-H", "-o", "value", "guid", pool_name])?;
        check_exit(&guid, "read installed pool identity")?;
        let uuid = runner.run(
            "blkid",
            &["-s", "PARTUUID", "-o", "value", &efi.to_string_lossy()],
        )?;
        check_exit(&uuid, "read installed EFI identity")?;
        let target = Self {
            pool_name: pool_name.into(),
            pool_guid: guid.stdout.trim().into(),
            dataset_prefix: dataset_prefix.into(),
            efi_partuuid: uuid.stdout.trim().into(),
        };
        target.validate()?;
        Ok(target)
    }

    pub fn validate(&self) -> Result<()> {
        let name = |s: &str| {
            !s.is_empty()
                && s.len() <= 255
                && (s.as_bytes()[0].is_ascii_alphanumeric() || s.starts_with('_'))
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
                && s != "."
                && s != ".."
        };
        ensure!(
            name(&self.pool_name) && name(&self.dataset_prefix),
            "Invalid installed dataset identity"
        );
        ensure!(
            self.pool_guid.parse::<u64>().is_ok_and(|n| n > 0),
            "Invalid pool GUID"
        );
        ensure!(
            !self.efi_partuuid.is_empty()
                && self.efi_partuuid.len() <= 64
                && self
                    .efi_partuuid
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() || b == b'-'),
            "Invalid EFI PARTUUID"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_path_and_option_injection() {
        let good = InstalledSystem {
            pool_name: "zroot".into(),
            pool_guid: "123".into(),
            dataset_prefix: "arch0".into(),
            efi_partuuid: "abcd-1234".into(),
        };
        assert!(good.validate().is_ok());
        for prefix in ["../root", "-a", "", "arch0/../../x"] {
            let mut bad = good.clone();
            bad.dataset_prefix = prefix.into();
            assert!(bad.validate().is_err());
        }
    }
}
