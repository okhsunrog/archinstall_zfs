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
            // Extract country name: everything before the 2-letter code
            // The format is "Country Name    XX    count"
            let parts: Vec<&str> = line.rsplitn(3, char::is_whitespace).collect();
            if parts.len() >= 3 {
                let name = parts[2].trim();
                if !name.is_empty() {
                    return Some(name.to_string());
                }
            }
            None
        })
        .collect()
}
