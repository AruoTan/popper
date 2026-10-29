Popper portable for Windows x64

Extract the whole ZIP to a writable folder and run popper.exe.
Keep portable.flag beside the executable. The config, logs and webview
directories are created beside popper.exe. Exit Popper before copying or upgrading.
For a manual upgrade, extract into a new folder and move config and logs into it.
WebView data can be recreated. The ZIP never contains a user profile.

Microsoft Edge WebView2 Runtime is required:
https://developer.microsoft.com/microsoft-edge/webview2/

To import installed settings, exit all Popper instances, then run the included
Import-InstalledSettings.ps1 in PowerShell before the first portable launch.
Only settings format v18 is accepted; older formats require reconfiguration.
It copies settings and BOTH local secret files from %APPDATA%/com.local.popper
without overwriting an existing config directory or changing the source.
For an already initialized profile, back up and move config first.
Treat backups of config as sensitive: they include the decryption key.

An absolute custom data directory can be selected with:
  .\popper.exe --data-dir "D:\Popper Data"
It takes precedence over portable.flag. Close the running Popper instance
before switching profiles; installed and portable modes share a single instance.

For Scoop, install popper.json from the GitHub Release. Its persist definition
links config and logs to Scoop's persist/popper/config and persist/popper/logs.
WebView data stays in the app version directory and is not persisted.
Ordinary uninstall retains persisted config and logs; a purge uninstall removes them.

User guide: https://github.com/AruoTan/popper/blob/main/USER_GUIDE.md
