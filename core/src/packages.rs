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

    // Sort: exact name match first, then starts-with, then contains
    let query_lower = query.to_lowercase();
    results.sort_by(|a, b| {
        let a_name = a.name.to_lowercase();
        let b_name = b.name.to_lowercase();
        let a_score = if a_name == query_lower {
            0
        } else if a_name.starts_with(&query_lower) {
            1
        } else {
            2
        };
        let b_score = if b_name == query_lower {
            0
        } else if b_name.starts_with(&query_lower) {
            1
        } else {
            2
        };
        a_score.cmp(&b_score).then(a_name.cmp(&b_name))
    });

    results.truncate(limit);
    Ok(results)
}

/// Search AUR packages via raur. Returns up to `limit` results sorted by popularity.
pub async fn search_aur(query: &str, limit: usize) -> Result<Vec<PackageInfo>> {
    use raur::Raur;

    let handle = raur::Handle::new();
    let results = handle
        .search(query)
        .await
        .map_err(|e| color_eyre::eyre::eyre!("AUR search failed: {e}"))?;

    let mut packages: Vec<PackageInfo> = results
        .into_iter()
        .map(|pkg| PackageInfo {
            name: pkg.name,
            version: pkg.version,
            description: pkg.description.unwrap_or_default(),
            repo: "aur".to_string(),
        })
        .collect();

    // raur results are already sorted by relevance, but let's cap
    packages.truncate(limit);
    Ok(packages)
}
