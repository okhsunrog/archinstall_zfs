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
    let search_terms = [query];

    for db in handle.syncdbs() {
        if let Ok(pkgs) = db.search(search_terms.iter().copied()) {
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

    // Sort by fuzzy match score against package name
    let mut scored: Vec<_> = results
        .into_iter()
        .map(|pkg| {
            let score = sublime_fuzzy::best_match(query, &pkg.name)
                .map(|m| m.score())
                .unwrap_or(0);
            (score, pkg)
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0));

    Ok(scored.into_iter().map(|(_, pkg)| pkg).take(limit).collect())
}

/// Search AUR packages via raur. Returns up to `limit` results sorted by popularity.
pub async fn search_aur(query: &str, limit: usize) -> Result<Vec<PackageInfo>> {
    use raur::Raur;

    let handle = raur::Handle::new();
    let results = handle
        .search(query)
        .await
        .map_err(|e| color_eyre::eyre::eyre!("AUR search failed: {e}"))?;

    let packages: Vec<PackageInfo> = results
        .into_iter()
        .map(|pkg| PackageInfo {
            name: pkg.name,
            version: pkg.version,
            description: pkg.description.unwrap_or_default(),
            repo: "aur".to_string(),
        })
        .collect();

    let mut scored: Vec<_> = packages
        .into_iter()
        .map(|pkg| {
            let score = sublime_fuzzy::best_match(query, &pkg.name)
                .map(|m| m.score())
                .unwrap_or(0);
            (score, pkg)
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0));

    Ok(scored.into_iter().map(|(_, pkg)| pkg).take(limit).collect())
}
