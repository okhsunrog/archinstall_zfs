//! Resolve package and group names against this machine's repositories.
//! `cargo run -p archinstall-zfs-core --example resolve_packages -- kde-applications firefox nosuchpkg`
fn main() -> color_eyre::Result<()> {
    let names: Vec<String> = std::env::args().skip(1).collect();
    let handle = archinstall_zfs_core::kernel::init_alpm()?;
    for name in &names {
        match archinstall_zfs_core::system::alpm_pacman::resolve_name(&handle, name) {
            Ok(pkgs) => println!("{name}: {} package(s)", pkgs.len()),
            Err(error) => println!("{name}: {error}"),
        }
    }
    Ok(())
}
