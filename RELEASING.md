# Releasing Dusk

Releases are built and published by GitHub Actions ([release.yml](.github/workflows/release.yml)) when a version tag is pushed. Version numbers follow [semantic versioning](https://semver.org): from 1.0.0 on, an incompatible change to the CLI (commands, exit codes), the settings file or the pipe protocol needs a new major version; new features are minor versions, fixes are patch versions.

## Checklist

1. **Main is green.** Push `main` and wait for the [CI workflow](.github/workflows/ci.yml) to pass. CI uses the MSVC toolchain; local builds often use GNU (see README), so CI is the real check.
2. **Docs are current.** [DEV_STATUS.md](DEV_STATUS.md) says what the release contains and what was accepted; README describes the released behavior.
3. **Set the version** everywhere it is recorded (workspace `Cargo.toml`, C# `Directory.Build.props`, the PowerToys Run `plugin.json`, the Command Palette `AppxManifest.xml`):

   ```powershell
   python scripts/set_version.py 1.2.3
   python scripts/set_version.py --check 1.2.3
   git commit -am "Release 1.2.3"
   git push origin main
   ```

   Skip this for the first release of a version that is already set (1.0.0 is set).
4. **Tag and push the tag:**

   ```powershell
   git tag v1.2.3
   git push origin v1.2.3
   ```

5. **Watch the Release workflow** (GitHub > Actions). It checks that the tag matches the sources, runs the tests, builds the release executables and both integrations, tests the C# client against the release daemon, and publishes `Dusk-1.2.3-x64.zip` with its `.sha256` on the [Releases page](https://github.com/Einhorntom/Dusk/releases), with notes generated from the commits.
6. **Check the published release:** download the zip, check its SHA-256, extract it, run `duskd.exe` and `dusk version` (shows the daemon version and protocol), and install the integrations from the extracted folder:

   ```powershell
   powershell -ExecutionPolicy Bypass -File .\integrations\install-powertoys-run.ps1
   powershell -ExecutionPolicy Bypass -File .\integrations\install-command-palette.ps1
   ```

## If something goes wrong

- **The workflow fails before publishing:** fix the cause on `main`, then move the tag to the fixed commit: `git tag -d v1.2.3`, `git push origin :refs/tags/v1.2.3`, tag again and push.
- **A published release is broken:** do not replace its files; publish a fixed patch version (1.2.4).

## Building a package locally

`scripts/package_release.ps1` is the packaging step of the workflow and also runs locally, once the release executables and both integrations are built:

```powershell
cargo build --release --bins
dotnet build integrations/PowerToysRun/Dusk.PowerToysRun.csproj -c Release -p:Platform=x64
dotnet build integrations/CommandPalette/Dusk.CommandPalette.csproj -c Release -p:Platform=x64
powershell -ExecutionPolicy Bypass -File scripts\package_release.ps1 -Version 1.2.3
```

The zip is written to `dist/` (ignored by git). A running Command Palette extension that was registered from the repository locks its build folder; quit it first, or package from a copy of the repository.
