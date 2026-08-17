use std::path::Path;

use color_eyre::eyre::Result;

use crate::system::cmd::{CommandRunner, check_exit};

/// Configure pacman mirrors on the target using reflector with country filters.
/// Runs reflector directly (not chrooted) and writes to target's mirrorlist.
pub fn configure_mirrors(
    runner: &dyn CommandRunner,
    target: &Path,
    countries: &[String],
) -> Result<()> {
    if countries.is_empty() {
        return Ok(());
    }

    tracing::info!(?countries, "configuring mirrors with reflector");

    let mirrorlist = format!("{}/etc/pacman.d/mirrorlist", target.display());

    let mut args: Vec<&str> = vec![
        "--latest",
        "20",
        "--protocol",
        "https",
        "--sort",
        "rate",
        "--save",
        &mirrorlist,
    ];

    // Add --country for each region
    for country in countries {
        args.push("--country");
        args.push(country);
    }

    let output = runner.run("reflector", &args)?;
    check_exit(&output, "reflector (mirror config)")?;

    tracing::info!("mirrors configured for target");
    Ok(())
}

/// List available reflector countries by running `reflector --list-countries`.
/// Returns a sorted list of country names.
pub fn list_mirror_countries(runner: &dyn CommandRunner) -> Vec<String> {
    let output = match runner.run("reflector", &["--list-countries"]) {
        Ok(o) if o.success() => o,
        _ => return Vec::new(),
    };

    // reflector --list-countries outputs lines like:
    //   Australia       AU     25
    //   Austria         AT     12
    // We want the country name (first column before the 2-letter code).
    output
        .stdout
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            // Skip header lines and empty lines
            if line.is_empty() || line.starts_with('-') || line.starts_with("Country") {
                return None;
            }
            // Extract the country name: everything before the two-letter code
            // and the count. The columns are padded to align, so split on runs
            // of whitespace — splitting on individual characters leaves the
            // padding behind and returns "Australia              AU".
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() < 3 {
                return None;
            }
            let name = fields[..fields.len() - 2].join(" ");
            (!name.is_empty()).then_some(name)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::cmd::tests::{CannedResponse, RecordingRunner};

    #[test]
    fn no_countries_means_no_reflector_run() {
        let runner = RecordingRunner::new(vec![]);
        configure_mirrors(&runner, Path::new("/mnt"), &[]).unwrap();
        assert!(runner.calls().is_empty());
    }

    #[test]
    fn each_country_becomes_its_own_flag() {
        let runner = RecordingRunner::new(vec![CannedResponse::default()]);
        configure_mirrors(
            &runner,
            Path::new("/mnt"),
            &["Germany".to_string(), "France".to_string()],
        )
        .unwrap();

        let calls = runner.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].program, "reflector");
        assert_eq!(
            calls[0].args.iter().filter(|a| *a == "--country").count(),
            2
        );
        assert!(calls[0].args.contains(&"Germany".to_string()));
        assert!(calls[0].args.contains(&"France".to_string()));
        // The mirrorlist must be written inside the target, not on the host.
        assert!(
            calls[0]
                .args
                .contains(&"/mnt/etc/pacman.d/mirrorlist".to_string())
        );
    }

    #[test]
    fn a_failing_reflector_stops_the_install() {
        let runner = RecordingRunner::new(vec![CannedResponse {
            exit_code: 1,
            stderr: "no mirrors matched".into(),
            ..Default::default()
        }]);
        let err = configure_mirrors(&runner, Path::new("/mnt"), &["Nowhere".to_string()])
            .expect_err("a mirrorlist that was not written must not pass silently");
        assert!(err.to_string().contains("reflector"));
    }

    #[test]
    fn country_names_are_taken_from_the_listing() {
        let runner = RecordingRunner::new(vec![CannedResponse {
            stdout: "Country                Code    Count\n\
                     -------                ----    -----\n\
                     Australia              AU         25\n\
                     Bosnia and Herzegovina BA          1\n\
                     United States          US        180\n"
                .into(),
            ..Default::default()
        }]);

        let countries = list_mirror_countries(&runner);

        // Multi-word names must survive: the code splits from the right.
        assert_eq!(
            countries,
            vec!["Australia", "Bosnia and Herzegovina", "United States"]
        );
    }

    #[test]
    fn a_failing_listing_yields_no_countries() {
        let runner = RecordingRunner::new(vec![CannedResponse {
            exit_code: 127,
            ..Default::default()
        }]);
        assert!(list_mirror_countries(&runner).is_empty());
    }
}
