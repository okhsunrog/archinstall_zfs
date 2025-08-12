from __future__ import annotations

from abc import ABC, abstractmethod
from pathlib import Path


class InitramfsHandler(ABC):
    """Abstract base class for initramfs handlers."""

    def __init__(self, target: Path, encryption_enabled: bool = False) -> None:
        self.target: Path = target
        self.encryption_enabled: bool = bool(encryption_enabled)

    @abstractmethod
    def configure(self) -> None:
        """Configure the initramfs system inside the target root."""

    @abstractmethod
    def generate_initramfs(self, kernel: str) -> bool:
        """Generate initramfs for a specific kernel inside the target root."""

    @abstractmethod
    def install_packages(self) -> list[str]:
        """Return required packages for this initramfs implementation."""

    @abstractmethod
    def setup_hooks(self) -> None:
        """Install any pacman hooks or auxiliary scripts to the target root."""
