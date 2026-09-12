use color_eyre::eyre::Result;

/// A package found by search.
#[derive(Debug, Clone)]
pub struct PackageInfo {
    pub name: String,
    pub version: String,
    pub description: String,
    pub repo: String, // e.g. "extra", "core", "aur"
}

/// Search official repo packages via alpm. Runs in a blocking task.
/// Returns up to `limit` results sorted by relevance (name match first).
pub async fn search_repo(query: &str, limit: usize) -> Result<Vec<PackageInfo>> {
    let query = query.to_string();
    tokio::task::spawn_blocking(move || search_repo_sync(&query, limit)).await?
}

fn search_repo_sync(query: &str, limit: usize) -> Result<Vec<PackageInfo>> {
    let handle = crate::kernel::init_alpm()?;

    let mut results = Vec::new();
    // alpm ANDs its terms, so "visual studio code" finds a package whose
    // description carries the words in any order.
    let search_terms = query_terms(query);

    for db in handle.syncdbs() {
        if let Ok(pkgs) = db.search(search_terms.iter().map(String::as_str)) {
            let repo_name = std::str::from_utf8(db.name().as_bytes()).unwrap_or("?");
            for pkg in pkgs {
                let name = std::str::from_utf8(pkg.name().as_bytes())
                    .unwrap_or("")
                    .to_string();
                let version = pkg.version().to_string();
                let description = pkg
                    .desc()
                    .map(|d| std::str::from_utf8(d.as_bytes()).unwrap_or(""))
                    .unwrap_or("")
                    .to_string();
                results.push(PackageInfo {
                    name,
                    version,
                    description,
                    repo: repo_name.to_string(),
                });
            }
        }
    }

    Ok(best_matches(query, results, limit))
}

/// The words of a query, for a search that combines them.
fn query_terms(query: &str) -> Vec<String> {
    query.split_whitespace().map(str::to_string).collect()
}

/// The forms of `query` worth sending to a search that takes one string:
/// as typed, and with the words joined by hyphens the way package names
/// are. "google chrome" is not a substring of `google-chrome` or of its
/// description, so the AUR finds it only in the second form.
fn query_forms(query: &str) -> Vec<String> {
    let terms = query_terms(query);
    let mut forms = vec![query.trim().to_string()];
    if terms.len() > 1 {
        forms.push(terms.join("-"));
    }
    forms.retain(|f| !f.is_empty());
    forms.dedup();
    forms
}

/// Order packages by how well their names fuzzy-match `query`, best first,
/// keeping `limit` of them. The sort is stable, so packages with equal
/// scores keep the order the search returned them in.
fn best_matches(query: &str, packages: Vec<PackageInfo>, limit: usize) -> Vec<PackageInfo> {
    let mut scored: Vec<_> = packages
        .into_iter()
        .map(|pkg| {
            let score = sublime_fuzzy::best_match(query, &pkg.name)
                .map(|m| m.score())
                .unwrap_or(0);
            (score, pkg)
        })
        .collect();
    scored.sort_by_key(|s| std::cmp::Reverse(s.0));
    scored.into_iter().map(|(_, pkg)| pkg).take(limit).collect()
}

/// Search AUR packages via raur. Returns up to `limit` results sorted by popularity.
pub async fn search_aur(query: &str, limit: usize) -> Result<Vec<PackageInfo>> {
    use raur::Raur;

    let handle = raur::Handle::new();
    let mut packages: Vec<PackageInfo> = Vec::new();
    for form in query_forms(query) {
        let results = handle
            .search(&form)
            .await
            .map_err(|e| color_eyre::eyre::eyre!("AUR search failed: {e}"))?;
        for pkg in results {
            if packages.iter().any(|p| p.name == pkg.name) {
                continue;
            }
            packages.push(PackageInfo {
                name: pkg.name,
                version: pkg.version,
                description: pkg.description.unwrap_or_default(),
                repo: "aur".to_string(),
            });
        }
    }

    Ok(best_matches(query, packages, limit))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_with_spaces_is_also_tried_as_a_package_name() {
        assert_eq!(
            query_forms("google chrome"),
            ["google chrome", "google-chrome"]
        );
        assert_eq!(query_forms("  chrome "), ["chrome"]);
        assert_eq!(
            query_terms("visual studio code"),
            ["visual", "studio", "code"]
        );
        assert!(query_forms("   ").is_empty());
    }

    fn pkg(name: &str) -> PackageInfo {
        PackageInfo {
            name: name.into(),
            version: "1".into(),
            description: String::new(),
            repo: "extra".into(),
        }
    }

    #[test]
    fn closer_names_come_first_and_the_limit_is_applied() {
        let ranked = best_matches(
            "zfs",
            vec![pkg("libzfs-tools"), pkg("zfs-utils"), pkg("unrelated")],
            2,
        );
        let names: Vec<&str> = ranked.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["zfs-utils", "libzfs-tools"]);
    }
}
