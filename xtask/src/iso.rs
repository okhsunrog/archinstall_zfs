use std::fs;
use std::path::Path;

use minijinja::Environment;

pub fn render_profile(
    profile_dir: &Path,
    out_dir: &Path,
    kernel: &str,
    zfs_mode: &str,
    headers: &str,
    fast_build: bool,
) -> Result<(), String> {
    if !profile_dir.exists() {
        return Err(format!(
            "Profile directory not found: {}",
            profile_dir.display()
        ));
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
        fs::remove_dir_all(out_dir)
            .map_err(|e| format!("failed to clean output directory: {e}"))?;
    }
    fs::create_dir_all(out_dir).map_err(|e| format!("create out_dir: {e}"))?;

    // Collect all templates first for minijinja's environment
    let mut env = Environment::new();
    let mut templates: Vec<(String, String)> = Vec::new();

    collect_templates(profile_dir, profile_dir, &mut templates)?;
    for (name, source) in &templates {
        env.add_template_owned(name.clone(), source.clone())
            .map_err(|e| format!("failed to parse template {name}: {e}"))?;
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
        let target = fs::read_link(src).map_err(|e| format!("read symlink: {e}"))?;
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("create dir: {e}"))?;
        }
        let _ = std::os::unix::fs::symlink(&target, &dst);
    }

    eprintln!("{}", out_dir.display());
    Ok(())
}

fn collect_templates(
    root: &Path,
    dir: &Path,
    templates: &mut Vec<(String, String)>,
) -> Result<(), String> {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .map_err(|e| format!("read dir: {e}"))?
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(|e| format!("read dir entry: {e}"))?;
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
                .map_err(|e| format!("failed to read template {}: {e}", path.display()))?;
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
) -> Result<(), String> {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .map_err(|e| format!("read dir: {e}"))?
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(|e| format!("read dir entry: {e}"))?;
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
            fs::create_dir_all(&dst).map_err(|e| format!("create dir: {e}"))?;
            walk_and_render(root, &src, out_dir, env, ctx, symlinks)?;
            continue;
        }

        if src.extension().is_some_and(|ext| ext == "j2") {
            let dst = dst.with_extension("");
            let template_name = rel.to_string_lossy().to_string();
            let tmpl = env
                .get_template(&template_name)
                .map_err(|e| format!("template not found {template_name}: {e}"))?;
            let rendered = tmpl
                .render(ctx)
                .map_err(|e| format!("failed to render {template_name}: {e}"))?;

            if rendered.trim().is_empty() {
                continue;
            }

            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("create dir: {e}"))?;
            }
            let mut content = rendered;
            if !content.ends_with('\n') {
                content.push('\n');
            }
            fs::write(&dst, content).map_err(|e| format!("write file: {e}"))?;
        } else {
            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("create dir: {e}"))?;
            }
            fs::copy(&src, &dst).map_err(|e| format!("copy file: {e}"))?;
        }
    }
    Ok(())
}
