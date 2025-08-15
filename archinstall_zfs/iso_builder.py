from __future__ import annotations

import argparse
import os
import shutil
import sys
from collections.abc import Iterable
from dataclasses import dataclass
from pathlib import Path

from jinja2 import Environment, FileSystemLoader, StrictUndefined


@dataclass
class BuildOptions:
    profile_dir: Path
    out_dir: Path
    kernel: str
    zfs_mode: str  # "precompiled" | "dkms"
    include_headers: bool | None  # None -> auto
    strict_links: bool
    deref_symlinks: bool
    fast_build: bool


def compute_headers_package(kernel: str) -> str:
    # linux -> linux-headers; linux-lts -> linux-lts-headers; linux-zen -> linux-zen-headers
    return f"{kernel}-headers"


def build_context(opts: BuildOptions) -> dict:
    use_dkms = opts.zfs_mode == "dkms"
    use_precompiled = not use_dkms
    include_headers = opts.include_headers if opts.include_headers is not None else use_dkms
    headers_pkg = compute_headers_package(opts.kernel)

    return {
        "kernel": opts.kernel,
        "use_dkms": use_dkms,
        "use_precompiled_zfs": use_precompiled,
        "include_headers": include_headers,
        "headers": headers_pkg,
        "fast_build": opts.fast_build,
    }


def walk_all_paths(root: Path) -> Iterable[Path]:
    # Deterministic traversal order
    for base, dirnames, filenames in os.walk(root, topdown=True, followlinks=False):
        # Sort for reproducibility
        dirnames.sort()
        filenames.sort()
        base_path = Path(base)
        for name in dirnames + filenames:
            yield base_path / name


def is_template_file(path: Path) -> bool:
    return path.suffix == ".j2"


def render_template(env: Environment, src_file: Path, dst_file: Path, ctx: dict) -> bool:
    # Render and write only if non-empty after strip; return True if written
    rel_template = str(src_file)
    # Convert to template name relative to loader root
    # The loader is rooted at profile_dir; so use relative POSIX path
    # to support Windows path separators if needed.
    loader = env.loader
    assert isinstance(loader, FileSystemLoader), "Expected FileSystemLoader"
    rel_template = src_file.relative_to(Path(loader.searchpath[0])).as_posix()
    template = env.get_template(rel_template)
    rendered: str = template.render(**ctx)
    if rendered.strip() == "":
        return False
    dst_file.parent.mkdir(parents=True, exist_ok=True)
    # Preserve trailing newline if template produced some content
    with open(dst_file, "w", encoding="utf-8", newline="\n") as f:
        f.write(rendered)
        if not rendered.endswith("\n"):
            f.write("\n")
    return True


def stage_profile(opts: BuildOptions) -> None:
    src_root = opts.profile_dir.resolve()
    dst_root = opts.out_dir.resolve()
    if dst_root.exists():
        # Clean destination to avoid stale files
        shutil.rmtree(dst_root)
    dst_root.mkdir(parents=True, exist_ok=True)

    env = Environment(
        loader=FileSystemLoader(str(src_root)),
        undefined=StrictUndefined,
        autoescape=False,  # We're generating shell scripts, not HTML  # noqa: S701
        keep_trailing_newline=True,
        lstrip_blocks=False,
        trim_blocks=False,
    )
    ctx = build_context(opts)

    # Pass 1: create directories and copy/render regular files (skip symlinks)
    symlink_paths: list[Path] = []
    for src in walk_all_paths(src_root):
        rel = src.relative_to(src_root)
        dst = dst_root / rel

        try:
            src.lstat()
        except FileNotFoundError:
            continue

        if os.path.islink(src):
            symlink_paths.append(src)
            continue

        if src.is_dir():
            dst.mkdir(parents=True, exist_ok=True)
            continue

        # Files
        if is_template_file(src):
            # Render to filename without .j2 suffix
            dst = dst.with_suffix("")
            written = render_template(env, src, dst, ctx)
            if not written:
                # Skip creating an empty file
                continue
        else:
            dst.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(src, dst)

    # Pass 2: recreate symlinks
    for src in symlink_paths:
        rel = src.relative_to(src_root)
        dst = dst_root / rel
        target = os.readlink(src)

        # Optionally dereference into a real file
        if opts.deref_symlinks:
            # Resolve the link to a file and copy contents if possible
            try:
                real = (src.parent / target).resolve() if not os.path.isabs(target) else Path(target)
                if real.is_file():
                    dst.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copy2(real, dst)
                    continue
            except FileNotFoundError:
                if opts.strict_links:
                    raise
                # else fall through to create the dangling symlink or skip

        dst.parent.mkdir(parents=True, exist_ok=True)
        try:
            os.symlink(target, dst)
        except FileExistsError:
            # Already created; ignore
            pass
        except FileNotFoundError:
            # Dangling link target omitted or missing
            if opts.strict_links:
                raise
            # else skip creating the link
            continue


def parse_args(argv: list[str]) -> BuildOptions:
    parser = argparse.ArgumentParser(description="Render and stage an ArchISO profile with optional ZFS DKMS/precompiled toggles")
    parser.add_argument("--profile-dir", required=True, help="Source profile directory (templates may be in-place)")
    parser.add_argument("--out-dir", required=True, help="Destination directory to render/copy into")
    parser.add_argument(
        "--kernel",
        choices=["linux", "linux-lts", "linux-zen"],
        default="linux-lts",
        help="Kernel package to include in ISO",
    )
    parser.add_argument(
        "--zfs",
        dest="zfs_mode",
        choices=["precompiled", "dkms"],
        default="precompiled",
        help="Use precompiled archzfs kernel modules or zfs-dkms",
    )
    parser.add_argument(
        "--headers",
        choices=["auto", "true", "false"],
        default="auto",
        help="Include kernel headers package: auto (true when zfs=dkms), true, or false",
    )
    parser.add_argument("--strict-links", action="store_true", help="Fail on dangling symlinks")
    parser.add_argument(
        "--deref-symlinks",
        action="store_true",
        help="Copy the contents of symlink targets instead of creating links",
    )
    parser.add_argument(
        "--fast",
        action="store_true",
        help="Enable fast build mode (omit heavy packages and optional content)",
    )

    ns = parser.parse_args(argv)
    inc_headers: bool | None
    if ns.headers == "auto":
        inc_headers = None
    elif ns.headers == "true":
        inc_headers = True
    else:
        inc_headers = False

    return BuildOptions(
        profile_dir=Path(ns.profile_dir),
        out_dir=Path(ns.out_dir),
        kernel=ns.kernel,
        zfs_mode=ns.zfs_mode,
        include_headers=inc_headers,
        strict_links=ns.strict_links,
        deref_symlinks=ns.deref_symlinks,
        fast_build=ns.fast,
    )


def main() -> int:
    opts = parse_args(sys.argv[1:])
    
    # Validate kernel/ZFS compatibility for DKMS builds
    if opts.zfs_mode == "dkms":
        try:
            from archinstall_zfs.validation import validate_kernel_zfs_compatibility
        except ImportError:
            # Fallback for standalone execution
            import sys as system
            import os
            system.path.insert(0, os.path.dirname(os.path.dirname(__file__)))
            from archinstall_zfs.validation import validate_kernel_zfs_compatibility
        
        is_compatible, warnings = validate_kernel_zfs_compatibility(opts.kernel, opts.zfs_mode)
        
        if warnings:
            for warning in warnings:
                print(f"WARNING: {warning}", file=sys.stderr)
        
        if not is_compatible:
            print(f"ERROR: Kernel {opts.kernel} is not compatible with ZFS DKMS.", file=sys.stderr)
            print("The ISO build would fail during DKMS module compilation.", file=sys.stderr)
            print("Please choose a different kernel or use precompiled ZFS modules.", file=sys.stderr)
            return 1
    
    stage_profile(opts)
    print(str(opts.out_dir))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
