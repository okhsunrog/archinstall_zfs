from archinstall.lib.output import info, debug

def main():
    info("Welcome to ArchInstall ZFS!")
    debug("Debug mode enabled")
    
    # Test importing key archinstall components
    from archinstall.lib.disk import get_block_devices
    from archinstall.lib.general import SysCommand
    
    disks = get_block_devices()
    debug(f"Found disks: {list(disks.keys())}")

if __name__ == "__main__":
    main()

