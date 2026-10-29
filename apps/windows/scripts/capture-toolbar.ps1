param(
  [Parameter(Mandatory = $true)][string]$LogPath,
  [Parameter(Mandatory = $true)][string]$OutputDirectory
)

Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

public sealed class PopperWindowInfo {
    public IntPtr Handle;
    public uint ProcessId;
    public string Title;
    public int Left;
    public int Top;
    public int Width;
    public int Height;
}

public static class PopperWindowProbe {
    private delegate bool EnumWindowsProc(IntPtr hwnd, IntPtr parameter);
    [StructLayout(LayoutKind.Sequential)]
    private struct RECT { public int Left, Top, Right, Bottom; }

    [DllImport("user32.dll")] private static extern bool EnumWindows(EnumWindowsProc callback, IntPtr parameter);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint processId);
    [DllImport("user32.dll")] private static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] private static extern int GetWindowTextW(IntPtr hwnd, StringBuilder text, int count);
    [DllImport("user32.dll")] private static extern bool GetWindowRect(IntPtr hwnd, out RECT rectangle);

    public static PopperWindowInfo[] VisibleWindows(uint[] processIds) {
        var wanted = new HashSet<uint>(processIds);
        var result = new List<PopperWindowInfo>();
        EnumWindows(delegate(IntPtr hwnd, IntPtr parameter) {
            uint processId;
            RECT rectangle;
            GetWindowThreadProcessId(hwnd, out processId);
            if (!wanted.Contains(processId) || !IsWindowVisible(hwnd) || !GetWindowRect(hwnd, out rectangle)) return true;
            var title = new StringBuilder(512);
            GetWindowTextW(hwnd, title, title.Capacity);
            result.Add(new PopperWindowInfo {
                Handle = hwnd,
                ProcessId = processId,
                Title = title.ToString(),
                Left = rectangle.Left,
                Top = rectangle.Top,
                Width = rectangle.Right - rectangle.Left,
                Height = rectangle.Bottom - rectangle.Top
            });
            return true;
        }, IntPtr.Zero);
        return result.ToArray();
    }
}
'@

$deadline = [DateTime]::UtcNow.AddMinutes(5)
while ([DateTime]::UtcNow -lt $deadline) {
  if ((Test-Path -LiteralPath $LogPath) -and (Select-String -LiteralPath $LogPath -SimpleMatch 'focus committed' -Quiet)) {
    Start-Sleep -Milliseconds 250
    $processIds = @(Get-Process popper -ErrorAction SilentlyContinue | ForEach-Object { [uint32]$_.Id })
    $windows = [PopperWindowProbe]::VisibleWindows($processIds)
    $index = 0
    foreach ($window in $windows) {
      if ($window.Width -lt 100 -or $window.Height -lt 20) { continue }
      $bitmap = New-Object System.Drawing.Bitmap($window.Width, $window.Height)
      $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
      try {
        $graphics.CopyFromScreen($window.Left, $window.Top, 0, 0, $bitmap.Size)
        $path = Join-Path $OutputDirectory ("toolbar-window-{0}-{1}x{2}.png" -f $index, $window.Width, $window.Height)
        $bitmap.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
        [pscustomobject]@{ Path = $path; PID = $window.ProcessId; HWND = $window.Handle; Title = $window.Title; Left = $window.Left; Top = $window.Top; Width = $window.Width; Height = $window.Height } | ConvertTo-Json -Compress | Add-Content -LiteralPath "$LogPath.windows.jsonl"
      } finally {
        $graphics.Dispose()
        $bitmap.Dispose()
      }
      $index += 1
    }
    exit 0
  }
  Start-Sleep -Milliseconds 50
}
exit 1
