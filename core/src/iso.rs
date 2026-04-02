use std::fs;
use std::path::Path;

use color_eyre::eyre::{Context, Result, bail};
use minijinja::Environment;

pub fn render_profile(
    profile_dir: &Path,
    out_dir: &Path,
    kernel: &str,
    zfs_mode: &str,
    headers: &str,
    fast_build: bool,
) -> Result<()> {
    if !profile_dir.exists() {
        bail!("Profile directory not found: {}", profile_dir.display());
    }

    let use_dkms = zfs_mode == "dkms";
    let include_headers = match headers {
        "true" => true,
        "false" => false,
        _ => use_dkms, // auto
    };
    let headers_pkg = format!("{kernel}-headers");

    // Clean output directory
    if out_dir.exists() {
        fs::remove_dir_all(out_dir).wrap_err("failed to clean output directory")?;
    }
    fs::create_dir_all(out_dir)?;

    // Collect all templates first for minijinja's environment
    let mut env = Environment::new();
    let mut templates: Vec<(String, String)> = Vec::new();

    collect_templates(profile_dir, profile_dir, &mut templates)?;
    for (name, source) in &templates {
        env.add_template_owned(name.clone(), source.clone())
            .wrap_err_with(|| format!("failed to parse template: {name}"))?;
    }

    // Build template context
    let ctx = minijinja::context! {
        kernel => kernel,
        use_dkms => use_dkms,
        use_precompiled_zfs => !use_dkms,
        include_headers => include_headers,
        headers => headers_pkg,
        fast_build => fast_build,
    };

    // Walk source directory and render/copy
    let mut symlinks: Vec<std::path::PathBuf> = Vec::new();
    walk_and_render(profile_dir, profile_dir, out_dir, &env, &ctx, &mut symlinks)?;

    // Recreate symlinks
    for src in &symlinks {
        let rel = src.strip_prefix(profile_dir).unwrap();
        let dst = out_dir.join(rel);
        let target = fs::read_link(src)?;
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        let _ = std::os::unix::fs::symlink(&target, &dst);
    }

    eprintln!("{}", out_dir.display());
    Ok(())
}

fn collect_templates(root: &Path, dir: &Path, templates: &mut Vec<(String, String)>) -> Result<()> {
    let mut entries: Vec<_> = fs::read_dir(dir)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let path = entry.path();
        if path.is_symlink() {
            continue;
        }
        if path.is_dir() {
            collect_templates(root, &path, templates)?;
        } else if path.extension().is_some_and(|ext| ext == "j2") {
            let rel = path.strip_prefix(root).unwrap();
            let name = rel.to_string_lossy().to_string();
            let source = fs::read_to_string(&path)
                .wrap_err_with(|| format!("failed to read template: {}", path.display()))?;
            templates.push((name, source));
        }
    }
    Ok(())
}

fn walk_and_render(
    root: &Path,
    dir: &Path,
    out_dir: &Path,
    env: &Environment,
    ctx: &minijinja::Value,
    symlinks: &mut Vec<std::path::PathBuf>,
) -> Result<()> {
    let mut entries: Vec<_> = fs::read_dir(dir)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let src = entry.path();
        let rel = src.strip_prefix(root).unwrap();
        let dst = out_dir.join(rel);

        if src.is_symlink() {
            symlinks.push(src);
            continue;
        }

        if src.is_dir() {
            fs::create_dir_all(&dst)?;
            walk_and_render(root, &src, out_dir, env, ctx, symlinks)?;
            continue;
        }

        // Regular file
        if src.extension().is_some_and(|ext| ext == "j2") {
            // Render template, output without .j2 suffix
            let dst = dst.with_extension("");
            let template_name = rel.to_string_lossy().to_string();
            let tmpl = env
                .get_template(&template_name)
                .wrap_err_with(|| format!("template not found: {template_name}"))?;
            let rendered = tmpl
                .render(ctx)
                .wrap_err_with(|| format!("failed to render: {template_name}"))?;

            // Skip empty renders
            if rendered.trim().is_empty() {
                continue;
            }

            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut content = rendered;
            if !content.ends_with('\n') {
                content.push('\n');
            }
            fs::write(&dst, content)?;
        } else {
            // Copy static file
            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_simple_template() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        let out = dir.path().join("out");
        fs::create_dir_all(&src).unwrap();

        // Write a template
        fs::write(
            src.join("test.conf.j2"),
            "kernel={{ kernel }}\ndkms={{ use_dkms }}\n",
        )
        .unwrap();

        // Write a static file
        fs::write(src.join("static.txt"), "unchanged\n").unwrap();

        render_profile(&src, &out, "linux-lts", "precompiled", "auto", false).unwrap();

        let rendered = fs::read_to_string(out.join("test.conf")).unwrap();
        assert!(rendered.contains("kernel=linux-lts"));
        assert!(rendered.contains("dkms=false"));

        let static_content = fs::read_to_string(out.join("static.txt")).unwrap();
        assert_eq!(static_content, "unchanged\n");
    }

    #[test]
    fn test_render_fast_build() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        let out = dir.path().join("out");
        fs::create_dir_all(&src).unwrap();

        fs::write(
            src.join("config.j2"),
            "{% if fast_build %}fast{% else %}full{% endif %}\n",
        )
        .unwrap();

        render_profile(&src, &out, "linux", "dkms", "auto", true).unwrap();
        let content = fs::read_to_string(out.join("config")).unwrap();
        assert!(content.contains("fast"));
    }

    #[test]
    fn test_empty_template_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        let out = dir.path().join("out");
        fs::create_dir_all(&src).unwrap();

        fs::write(
            src.join("maybe.conf.j2"),
            "{% if fast_build %}content{% endif %}",
        )
        .unwrap();

        render_profile(&src, &out, "linux-lts", "precompiled", "auto", false).unwrap();
        // Should not create the file since rendered content is empty
        assert!(!out.join("maybe.conf").exists());
    }
}
