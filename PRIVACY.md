# Dusk privacy policy

Dusk controls the displays connected to your PC. It does not collect, store or send any personal data.

- **No network access.** Dusk never connects to the internet or to any other computer. Its command line and PowerToys integrations talk to it through a local named pipe that rejects remote connections.
- **No telemetry, analytics or crash reporting.**
- **What stays on your PC:**
  - Your settings, presets and hotkeys, in `%APPDATA%\Dusk\config.toml`.
  - A diagnostic log, only if you turn it on (Settings > General). It goes to `%LOCALAPPDATA%\Dusk\logs` and contains monitor models and serial numbers but not your user name.
  - You can read, edit or delete these files at any time; they are not removed when you uninstall Dusk.
- **Monitor information.** Dusk reads your monitors' model names, serial numbers and settings over DDC/CI and Windows display APIs, only to control them. This information never leaves your PC.

Questions: open an issue at https://github.com/Einhorntom/Dusk/issues.
