use std::fs;
use std::path::Path;

use color_eyre::eyre::{Context, Result};

use crate::system::cmd::{check_exit, chroot, CommandRunner};

pub fn set_hostname(target: &Path, hostname: &str) -> Result<()> {
    let path = target.join("etc/hostname");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, format!("{hostname}\n")).wrap_err("failed to write hostname")?;
    tracing::info!(hostname, "set hostname");
    Ok(())
}

pub fn set_locale(runner: &dyn CommandRunner, target: &Path, locale: &str) -> Result<()> {
    // Uncomment the locale in locale.gen
    let locale_gen = target.join("etc/locale.gen");
    if locale_gen.exists() {
        let content = fs::read_to_string(&locale_gen)?;
        let uncommented = content.replace(&format!("#{locale}"), locale);
        fs::write(&locale_gen, uncommented)?;
    }

    // Run locale-gen
    let output = chroot(runner, target, "locale-gen")?;
    check_exit(&output, "locale-gen")?;

    // Write locale.conf
    let locale_conf = target.join("etc/locale.conf");
    let lang = locale.split_whitespace().next().unwrap_or(locale);
    fs::write(&locale_conf, format!("LANG={lang}\n")).wrap_err("failed to write locale.conf")?;

    tracing::info!(locale, "set locale");
    Ok(())
}

pub fn set_keyboard(runner: &dyn CommandRunner, target: &Path, layout: &str) -> Result<()> {
    let vconsole = target.join("etc/vconsole.conf");
    if let Some(parent) = vconsole.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&vconsole, format!("KEYMAP={layout}\n")).wrap_err("failed to write vconsole.conf")?;
    tracing::info!(layout, "set keyboard layout");
    Ok(())
}

pub fn set_timezone(target: &Path, timezone: &str) -> Result<()> {
    let localtime = target.join("etc/localtime");
    let zoneinfo = format!("/usr/share/zoneinfo/{timezone}");
    let target_zoneinfo = target.join(format!("usr/share/zoneinfo/{timezone}"));

    // Remove existing symlink
    let _ = fs::remove_file(&localtime);

    if let Some(parent) = localtime.parent() {
        fs::create_dir_all(parent)?;
    }

    // Create relative symlink
    std::os::unix::fs::symlink(&zoneinfo, &localtime)
        .wrap_err_with(|| format!("failed to symlink timezone: {timezone}"))?;

    tracing::info!(timezone, "set timezone");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_hostname() {
        let dir = tempfile::tempdir().unwrap();
        set_hostname(dir.path(), "archbox").unwrap();

        let content = fs::read_to_string(dir.path().join("etc/hostname")).unwrap();
        assert_eq!(content, "archbox\n");
    }

    #[test]
    fn test_set_keyboard() {
        let dir = tempfile::tempdir().unwrap();
        let runner = crate::system::cmd::tests::RecordingRunner::new(vec![]);
        set_keyboard(&runner, dir.path(), "de-latin1").unwrap();

        let content = fs::read_to_string(dir.path().join("etc/vconsole.conf")).unwrap();
        assert_eq!(content, "KEYMAP=de-latin1\n");
    }

    #[test]
    fn test_set_timezone() {
        let dir = tempfile::tempdir().unwrap();
        // Create etc dir
        fs::create_dir_all(dir.path().join("etc")).unwrap();
        set_timezone(dir.path(), "Europe/Berlin").unwrap();

        let localtime = dir.path().join("etc/localtime");
        assert!(localtime.is_symlink());
        let target = fs::read_link(&localtime).unwrap();
        assert_eq!(
            target.to_str().unwrap(),
            "/usr/share/zoneinfo/Europe/Berlin"
        );
    }
}
