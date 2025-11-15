# GitHub Actions Workflows

This directory contains GitHub Actions workflows for building, testing, and publishing the `par-term-emu-core-rust` package.

## Workflows

### publish-testpypi.yml - Publish to TestPyPI

**Trigger**:
- Manual (`workflow_dispatch`)
- Pull requests to main (build only, no publish)

**Purpose**: Builds and publishes to TestPyPI for testing before production release.

**Jobs**:
1. **build-test-wheels**: Builds wheels for all platforms (Linux x86_64/ARM64, macOS x86_64/universal2, Windows x86_64) - Python 3.13 only
2. **build-test-sdist**: Builds source distribution
3. **publish-to-testpypi**: Publishes to TestPyPI (manual trigger only)
4. **test-testpypi-install**: Verifies package can be installed from TestPyPI

**Features**:
- Automatic build verification on PRs
- TestPyPI publishing on manual trigger
- Installation testing after publish
- Discord notifications on success
- Skip existing packages

**Required Secrets**:
- `DISCORD_WEBHOOK`: Discord webhook URL for notifications

**TestPyPI Setup**:
- Environment: `testpypi`
- Package URL: https://test.pypi.org/p/par-term-emu-core-rust
- Uses TestPyPI trusted publishing

### deployment.yml - Build and Deploy

**Trigger**: Manual (`workflow_dispatch`)

**Purpose**: Builds Python wheels for multiple platforms and publishes to PyPI.

**Jobs**:
1. **linux**: Builds wheels for Linux x86_64 and ARM64 (Python 3.11, 3.12, 3.13)
2. **macos**: Builds wheels for macOS x86_64 and universal2 (Python 3.11, 3.12, 3.13)
3. **windows**: Builds wheels for Windows x86_64 (Python 3.11, 3.12, 3.13)
4. **sdist**: Builds source distribution
5. **publish**: Publishes to PyPI and sends Discord notification

**Platform Coverage**:
| Platform | Architecture | Python Versions | Testing |
|----------|--------------|-----------------|---------|
| Linux | x86_64 | 3.11, 3.12, 3.13 | ✅ Full |
| Linux | ARM64 (aarch64) | 3.11, 3.12, 3.13 | ⚠️ Build only* |
| macOS | x86_64 | 3.11, 3.12, 3.13 | ✅ Full |
| macOS | universal2 (Intel + Apple Silicon) | 3.11, 3.12, 3.13 | ⚠️ Partial† |
| Windows | x86_64 | 3.11, 3.12, 3.13 | ✅ PTY tests skipped |

*ARM64 wheels built via QEMU cross-compilation, not tested on CI
†Universal2 tested on x86_64 runner only

**Features**:
- Cross-platform wheel building using maturin
- Automated testing on x86_64 platforms
- QEMU-based ARM64 cross-compilation
- PyPI trusted publishing (OIDC)
- Discord notifications on successful publish
- Skip existing packages on PyPI

**Required Secrets**:
- `DISCORD_WEBHOOK`: Discord webhook URL for notifications

**PyPI Setup**:
- Environment: `pypi`
- Package URL: https://pypi.org/p/par-term-emu-core-rust
- Uses PyPI trusted publishing (no token needed)

### release.yml - Create GitHub Release

**Trigger**: Manual (`workflow_dispatch`)

**Purpose**: Creates a GitHub release with signed artifacts and triggers PyPI publishing.

**Jobs**:
1. **build-wheels**: Builds wheels for all platforms (Linux x86_64/ARM64, macOS x86_64/universal2, Windows x86_64) with all Python versions (3.11, 3.12, 3.13)
2. **build-sdist**: Builds source distribution
3. **github-release**: Creates GitHub release with Sigstore signatures
4. **trigger-pypi-publish**: Triggers the deployment workflow for PyPI publishing

**Features**:
- Sigstore signing for all distribution artifacts
- Auto-generated release notes
- Automatic version extraction from `__version__`
- Discord notifications on release creation
- Automatic PyPI publishing trigger

**Required Secrets**:
- `DISCORD_WEBHOOK`: Discord webhook URL for notifications

**Release Process**:
1. Update version in `python/par_term_emu_core_rust/__init__.py` and `Cargo.toml`
2. Commit changes
3. Manually trigger the "Release 🐍 distribution" workflow
4. Workflow creates GitHub release with signed artifacts
5. Workflow automatically triggers PyPI publishing
6. Discord notifications sent for both release and PyPI publish

## Configuration

### Discord Webhook Setup

1. Create a Discord webhook in your server settings
2. Add the webhook URL as a repository secret named `DISCORD_WEBHOOK`:
   ```bash
   gh secret set DISCORD_WEBHOOK
   ```
3. Paste your webhook URL when prompted

### PyPI Trusted Publishing Setup

#### Production PyPI
1. Go to https://pypi.org/manage/account/publishing/
2. Add a new publisher:
   - **PyPI Project Name**: `par-term-emu-core-rust`
   - **Owner**: `{your-github-username-or-org}`
   - **Repository name**: `par-term-emu-core-rust`
   - **Workflow name**: `deployment.yml`
   - **Environment name**: `pypi`

#### TestPyPI (for testing)
1. Go to https://test.pypi.org/manage/account/publishing/
2. Add a new publisher:
   - **PyPI Project Name**: `par-term-emu-core-rust`
   - **Owner**: `{your-github-username-or-org}`
   - **Repository name**: `par-term-emu-core-rust`
   - **Workflow name**: `publish-testpypi.yml`
   - **Environment name**: `testpypi`

## Manual Workflow Triggers

### Test with TestPyPI (recommended first)
```bash
gh workflow run publish-testpypi.yml
```

After successful TestPyPI publish, test installation:
```bash
uv venv .venv
source .venv/bin/activate
uv pip install --index-url https://test.pypi.org/simple/ --extra-index-url https://pypi.org/simple/ par-term-emu-core-rust
python -c "from par_term_emu_core_rust import Terminal; print('Success!')"
```

### Deploy to PyPI Only
```bash
gh workflow run deployment.yml
```

### Create Release and Deploy (full release process)
```bash
# 1. Update version in both files
# 2. Commit changes
# 3. Trigger release
gh workflow run release.yml
```

## Version Management

The package version is managed in two places and must be kept in sync:
- `Cargo.toml`: Line 3 - `version = "0.2.0"`
- `python/par_term_emu_core_rust/__init__.py`: Line 31 - `__version__ = "0.2.0"`

Before creating a release:
1. Update both version strings to the new version
2. Commit the changes
3. Trigger the release workflow

## Workflow Permissions

### deployment.yml
- `id-token: write` - Required for PyPI trusted publishing
- `contents: read` - Default repository access

### release.yml
- `contents: write` - Create GitHub releases
- `id-token: write` - Sign artifacts with Sigstore
- `actions: write` - Trigger deployment workflow

## Troubleshooting

### Discord Notifications Not Sent
- Verify `DISCORD_WEBHOOK` secret is set correctly
- Check webhook URL is valid in Discord
- Notifications have `continue-on-error: true` so they won't fail the workflow

### PyPI Publishing Fails
- Verify trusted publisher is configured on PyPI
- Check environment name matches (`pypi`)
- Ensure package version doesn't already exist on PyPI
- Review PyPI publish logs in GitHub Actions

### Version Extraction Fails
- Ensure `__version__` is defined in `python/par_term_emu_core_rust/__init__.py`
- Verify Python syntax is valid in `__init__.py`
- Check Python version (3.13) is available in workflow

## Best Practices

1. **Always test locally before release**:
   ```bash
   make checkall
   cargo test
   make test-python
   ```

2. **Test on TestPyPI first**:
   - Publish to TestPyPI before production PyPI
   - Verify installation from TestPyPI works
   - Test in a clean environment

3. **Use semantic versioning**: MAJOR.MINOR.PATCH (e.g., 0.2.0)

4. **Create releases from main branch**: Ensure all changes are merged to main

5. **Review artifacts before publishing**: Check the wheels in the Actions artifacts

6. **Monitor Discord notifications**: Verify successful publishing via Discord

## Recommended Release Workflow

1. **Update version numbers**:
   ```bash
   # Update both files to new version (e.g., 0.3.0)
   # - Cargo.toml: version = "0.3.0"
   # - python/par_term_emu_core_rust/__init__.py: __version__ = "0.3.0"
   ```

2. **Test locally**:
   ```bash
   make checkall
   cargo test
   make test-python
   ```

3. **Commit and push**:
   ```bash
   git add Cargo.toml python/par_term_emu_core_rust/__init__.py
   git commit -m "chore: bump version to 0.3.0"
   git push
   ```

4. **Test on TestPyPI**:
   ```bash
   gh workflow run publish-testpypi.yml
   # Wait for completion, then test installation
   ```

5. **Create production release**:
   ```bash
   gh workflow run release.yml
   # This will:
   # - Create GitHub release with signed artifacts
   # - Automatically trigger PyPI publishing
   # - Send Discord notifications
   ```

## Resources

- [PyPI Trusted Publishing](https://docs.pypi.org/trusted-publishers/)
- [Sigstore](https://www.sigstore.dev/)
- [Maturin Action](https://github.com/PyO3/maturin-action)
- [Discord Webhooks](https://discord.com/developers/docs/resources/webhook)
