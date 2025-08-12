from __future__ import annotations

import json
from pathlib import Path
from typing import Any

from archinstall.lib.configuration import ConfigurationOutput

ZFS_CONFIG_KEY = "archinstall_zfs"
ZFS_CREDS_KEY = "archinstall_zfs"


def merge_zfs_into_config(config_json: str, zfs_block: dict[str, Any]) -> str:
    data = json.loads(config_json) if config_json else {}
    data[ZFS_CONFIG_KEY] = {"schema_version": 1, **zfs_block}
    return json.dumps(data, indent=4, sort_keys=True)


def extract_zfs_from_config(config_json: str) -> tuple[dict[str, Any], str]:
    data = json.loads(config_json) if config_json else {}
    zfs = data.pop(ZFS_CONFIG_KEY, {})
    return zfs, json.dumps(data, indent=4, sort_keys=True)


def save_combined_configuration(
    config_output: ConfigurationOutput,
    dest_path: Path,
    zfs_block: dict[str, Any],
    creds: bool = False,
    password: str | None = None,
) -> None:
    # Prepare base JSON strings
    user_cfg = config_output.user_config_to_json()
    combined_cfg = merge_zfs_into_config(user_cfg, zfs_block)

    # Write combined user_configuration.json
    (dest_path / config_output.user_configuration_file).write_text(combined_cfg)

    if creds:
        # Reuse archinstall's save for credentials then overwrite with merged if needed
        config_output.save_user_creds(dest_path, password=password)
        # If you want to also store ZFS secrets in creds, extend here


def load_combined_configuration(config_path: Path) -> tuple[dict[str, Any], dict[str, Any]]:
    """Return (archinstall_config_dict, zfs_block)."""
    data = json.loads(config_path.read_text()) if config_path.exists() else {}
    zfs = data.pop(ZFS_CONFIG_KEY, {})
    return data, zfs
