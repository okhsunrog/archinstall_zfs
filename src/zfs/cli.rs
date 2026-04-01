use color_eyre::eyre::Result;

use crate::system::cmd::{check_exit, CmdOutput, CommandRunner};

pub fn run_zfs(runner: &dyn CommandRunner, args: &[&str]) -> Result<CmdOutput> {
    tracing::debug!(?args, "zfs");
    runner.run("zfs", args)
}

pub fn run_zfs_json<T: serde::de::DeserializeOwned>(
    runner: &dyn CommandRunner,
    args: &[&str],
) -> Result<T> {
    let mut full_args = vec!["-j"];
    full_args.extend_from_slice(args);
    let output = runner.run("zfs", &full_args)?;
    check_exit(&output, &format!("zfs {}", args.join(" ")))?;
    let parsed: T = serde_json::from_str(&output.stdout)?;
    Ok(parsed)
}

pub fn run_zpool(runner: &dyn CommandRunner, args: &[&str]) -> Result<CmdOutput> {
    tracing::debug!(?args, "zpool");
    runner.run("zpool", args)
}

pub fn run_zpool_json<T: serde::de::DeserializeOwned>(
    runner: &dyn CommandRunner,
    args: &[&str],
) -> Result<T> {
    let mut full_args = vec!["-j"];
    full_args.extend_from_slice(args);
    let output = runner.run("zpool", &full_args)?;
    check_exit(&output, &format!("zpool {}", args.join(" ")))?;
    let parsed: T = serde_json::from_str(&output.stdout)?;
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::cmd::tests::{CannedResponse, RecordingRunner};

    #[test]
    fn test_run_zfs_json_prepends_j_flag() {
        let fixture = std::fs::read_to_string(format!(
            "{}/tests/fixtures/zfs_list.json",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();

        let runner = RecordingRunner::new(vec![CannedResponse {
            stdout: fixture,
            ..Default::default()
        }]);

        let result: super::super::models::ZfsListOutput = run_zfs_json(&runner, &["list"]).unwrap();
        assert!(result.datasets.contains_key("testpool"));

        let calls = runner.calls();
        assert_eq!(calls[0].program, "zfs");
        assert_eq!(calls[0].args[0], "-j");
        assert_eq!(calls[0].args[1], "list");
    }
}
