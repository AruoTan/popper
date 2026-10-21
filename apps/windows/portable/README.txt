Popper portable for Windows x64

Extract the whole ZIP to a writable folder and run popper.exe.
Keep portable.flag beside the executable. Data is stored in data/config,
data/logs and data/webview. Exit Popper before copying or upgrading it.
For a manual upgrade, extract into a new folder and move your existing data
folder into it. The ZIP never contains a user profile.

Microsoft Edge WebView2 Runtime is required:
https://developer.microsoft.com/microsoft-edge/webview2/

To import installed settings, exit all Popper instances, then run the included
Import-InstalledSettings.ps1 in PowerShell before the first portable launch.
It copies settings and BOTH local secret files from %APPDATA%/com.local.popper
without overwriting an existing config directory or changing the source.
For an already initialized profile, back up and move data/config first.
Treat backups of data/config as sensitive: they include the decryption key.

An absolute custom data directory can be selected with:
  .\popper.exe --data-dir "D:\Popper Data"
It takes precedence over portable.flag. Close the running Popper instance
before switching profiles; installed and portable modes share a single instance.

For Scoop, install popper.json from the GitHub Release. Its persist definition
links data to Scoop's persist/popper/data and keeps it across upgrades and
ordinary uninstall. A purge uninstall removes persisted data too.
