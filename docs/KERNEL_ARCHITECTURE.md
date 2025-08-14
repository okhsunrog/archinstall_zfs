# Enhanced Kernel Architecture for ZFS Installation

## Overview

The enhanced kernel architecture provides a unified, extensible system for managing kernel variants and their associated ZFS packages. This system addresses the previous limitations and provides proper fallback logic while maintaining full backward compatibility.

## Key Improvements

### 1. **Precompiled ZFS Support for All Kernels**

**Before:**
- Only `linux-lts` supported precompiled ZFS
- `linux` and `linux-zen` were forced to use DKMS

**After:**
- All kernel variants support precompiled ZFS:
  - `linux-lts` → `zfs-linux-lts`
  - `linux` → `zfs-linux`
  - `linux-zen` → `zfs-linux-zen`

### 2. **Consistent Fallback Logic**

**Before:**
```
linux-lts precompiled fails → linux + DKMS  ❌ (wrong kernel!)
```

**After:**
```
linux-lts precompiled fails → linux-lts + DKMS  ✅ (same kernel!)
```

### 3. **Centralized Configuration**

**Before:**
- Kernel logic scattered across multiple files
- Hard-coded package mappings
- Difficult to extend

**After:**
- Single `KernelRegistry` manages all variants
- Easy to add new kernels
- Clean separation of concerns

## Architecture Components

### KernelVariant

Defines a kernel variant and its associated packages:

```python
@dataclass
class KernelVariant:
    name: str                           # "linux-lts"
    display_name: str                   # "Linux LTS"
    kernel_package: str                 # "linux-lts"
    headers_package: str                # "linux-lts-headers"
    zfs_precompiled_package: str | None # "zfs-linux-lts"
    supports_precompiled: bool          # True
    is_default: bool = False            # True for linux-lts
```

### KernelRegistry

Central repository for all kernel variants:

```python
registry = get_kernel_registry()

# Get a specific variant
variant = registry.get_variant("linux-zen")

# Get all variants that support precompiled ZFS
precompiled_variants = registry.get_precompiled_variants()

# Register a custom variant
registry.register_variant(custom_variant)
```

### ZFSPackageManager

Handles package installation with intelligent fallback:

```python
manager = ZFSPackageManager(registry)

result = manager.install_zfs_packages(
    "linux-zen",
    ZFSModuleMode.PRECOMPILED,
    installation
)

if result.success:
    print(result.get_summary())
```

### EnhancedZFSInstaller

Orchestrates the installation process:

```python
installer = EnhancedZFSInstaller(registry)

result = installer.install_with_fallback(
    "linux-lts",
    ZFSModuleMode.PRECOMPILED,
    installation
)
```

## Default Kernel Variants

The system comes with three default kernel variants:

| Kernel | Display Name | Precompiled Package | Default |
|--------|--------------|-------------------|---------|
| `linux-lts` | Linux LTS | `zfs-linux-lts` | ✅ |
| `linux` | Linux | `zfs-linux` | ❌ |
| `linux-zen` | Linux Zen | `zfs-linux-zen` | ❌ |

## Fallback Logic

The fallback strategy follows this principle: **Never change kernel variant during fallback**.

### Precompiled Request

```
1. Try: linux-zen + precompiled ZFS
   ↓ (if fails)
2. Fallback: linux-zen + DKMS
```

### DKMS Request

```
1. Install: linux-lts + DKMS
```

## Menu Integration

The enhanced menu system automatically generates options for all supported kernels:

```
Linux LTS + precompiled ZFS (recommended)
Linux LTS + ZFS DKMS
Linux + precompiled ZFS          ← NEW!
Linux + ZFS DKMS
Linux Zen + precompiled ZFS      ← NEW!
Linux Zen + ZFS DKMS
```

## Extensibility

### Adding New Kernel Variants

#### Method 1: Programmatic Registration

```python
from archinstall_zfs.kernel import get_kernel_registry, KernelVariant

registry = get_kernel_registry()

custom_variant = KernelVariant(
    name="linux-hardened",
    display_name="Linux Hardened",
    kernel_package="linux-hardened",
    headers_package="linux-hardened-headers",
    zfs_precompiled_package="zfs-linux-hardened",
    supports_precompiled=True
)

registry.register_variant(custom_variant)
```

#### Method 2: Configuration File

Create `/etc/archinstall-zfs/kernel-variants.json`:

```json
{
  "kernel_variants": [
    {
      "name": "linux-hardened",
      "display_name": "Linux Hardened",
      "kernel_package": "linux-hardened",
      "headers_package": "linux-hardened-headers",
      "zfs_precompiled_package": "zfs-linux-hardened",
      "supports_precompiled": true,
      "is_default": false
    }
  ]
}
```

#### Method 3: Auto-Detection

The registry can automatically detect available kernels:

```python
registry = get_kernel_registry()
registry.auto_detect_variants()
```

## Backward Compatibility

### Existing Configurations

All existing configurations continue to work without changes:

```json
{
  "kernels": ["linux-lts"],
  "zfs_module_mode": "precompiled"
}
```

### Menu Selections

Previous menu selections are preserved and enhanced:

- `linux-lts` + precompiled: ✅ (works as before)
- `linux` + precompiled: ✅ (now available!)
- `linux-zen` + precompiled: ✅ (now available!)

### API Compatibility

All existing functions maintain their interfaces:

```python
# This continues to work
def install_zfs(self) -> bool:
    # Now uses enhanced system internally
```

## Error Handling

The enhanced system provides detailed error reporting:

```python
result = installer.install_with_fallback(...)

if not result.success:
    print(f"Installation failed: {result.get_summary()}")
    for error in result.errors:
        print(f"  - {error}")
```

## Testing

Comprehensive test suite validates:

- Kernel variant creation and validation
- Registry functionality
- Package manager behavior
- Fallback logic correctness
- Backward compatibility
- Error handling

Run tests with:

```bash
python -m pytest tests/test_kernel_system.py -v
```

## Migration Guide

### For Users

**No action required** - existing configurations continue to work and automatically benefit from:
- Enhanced fallback logic
- More kernel options with precompiled ZFS
- Better error messages

### For Developers

#### Updating Code

**Old:**
```python
if "lts" in kernel_version:
    package = "zfs-linux-lts"
else:
    package = "zfs-linux"
```

**New:**
```python
from archinstall_zfs.kernel import get_kernel_registry

registry = get_kernel_registry()
variant = registry.get_variant(kernel_name)
packages = variant.get_precompiled_packages()
```

#### Adding New Kernels

Instead of modifying multiple files, simply register a new variant:

```python
registry.register_variant(new_variant)
```

## Performance Impact

- **Minimal overhead**: Registry initialization is cached
- **Lazy loading**: Variants loaded only when needed
- **Efficient lookups**: O(1) variant retrieval
- **No breaking changes**: Existing code paths preserved

## Future Enhancements

### Planned Features

1. **Dynamic package detection**: Automatically detect available ZFS packages
2. **Version compatibility checking**: Ensure kernel/ZFS version compatibility
3. **Custom repository support**: Support for third-party ZFS packages
4. **Plugin system**: Allow external kernel variant providers

### Extension Points

The architecture is designed for future extensions:

- Custom package searchers
- Alternative fallback strategies
- Integration with package managers
- Support for non-Arch distributions

## Troubleshooting

### Common Issues

#### Kernel Not Found

```
Error: Unsupported kernel: linux-custom
```

**Solution:** Register the kernel variant or use auto-detection.

#### Precompiled Package Not Available

```
Warning: Precompiled package zfs-linux-zen not found, falling back to DKMS
```

**Solution:** This is expected behavior. The system automatically falls back to DKMS.

#### DKMS Build Failure

```
Error: DKMS installation failed: Missing headers
```

**Solution:** Ensure the correct headers package is available for your kernel.

### Debug Information

Enable debug logging to see detailed information:

```python
import logging
logging.getLogger('archinstall_zfs.kernel').setLevel(logging.DEBUG)
```

## Contributing

### Adding Support for New Kernels

1. Create a `KernelVariant` definition
2. Add tests for the new variant
3. Update documentation
4. Submit a pull request

### Reporting Issues

When reporting issues, include:

- Kernel variant being used
- ZFS module mode (precompiled/DKMS)
- Full error messages
- System information

## Conclusion

The enhanced kernel architecture provides a robust, extensible foundation for ZFS kernel management while maintaining full backward compatibility. Users benefit from more options and better reliability, while developers enjoy cleaner, more maintainable code.