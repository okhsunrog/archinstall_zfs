from importlib.resources import files as _files

from archinstall_zfs.main import main as main


def asset_path(relative: str) -> str:
    return str(_files(__package__) / relative)
