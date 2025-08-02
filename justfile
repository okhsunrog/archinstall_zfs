# Default recipe to display available commands
default:
    @just --list

# Format code with ruff
format:
    uv run ruff format .

# Lint and auto-fix with ruff
lint:
    uv run ruff check --fix .

# Type check with mypy
type-check:
    uv run mypy .

# Run tests with pytest
test:
    uv run pytest

# Run all quality checks
all: format lint type-check test

# Clean up cache and build artifacts
clean:
    rm -rf .mypy_cache/
    rm -rf .pytest_cache/
    rm -rf .ruff_cache/
    rm -rf htmlcov/
    rm -rf build/
    rm -rf dist/
    rm -rf *.egg-info/
    find . -type d -name __pycache__ -exec rm -rf {} +
    find . -type f -name "*.pyc" -delete

# Install development dependencies
install-dev:
    uv pip install -e .[dev]

# Setup development environment
setup: install-dev
    @echo "Development environment setup complete!"
    @echo "Run 'just all' to check code quality"

# Run a quick check (format + lint only)
quick: format lint

# Run tests with coverage report
test-cov:
    uv run pytest --cov=archinstall_zfs --cov-report=html --cov-report=term-missing

# Run tests with coverage report (XML format for CI)
test-cov-xml:
    uv run pytest --cov=archinstall_zfs --cov-report=xml --cov-report=html --cov-report=term-missing

# Check code without making changes
check:
    uv run ruff format --check .
    uv run ruff check .
    uv run mypy .