# Bounded native smoke for AgentPet (no extra deps).
# Build the candidate (debug, empty windows so setup owns the first WebView + CDP):
#   npx tauri build --debug --no-bundle --config src-tauri/qa.windows.json
#   powershell -NoProfile -ExecutionPolicy Bypass -File windows/scripts/native-qa.ps1
#   -Exe <path>  -OutDir <dir>  -DebugPort 9223  -MonitorBoundary  -TimeoutSec 90
# Isolation: AGENTPET_QA_PROFILE + AGENTPET_CDP_PORT (debug-only). Does not copy or restore %APPDATA%\AgentPet.
param(
  [string]$Exe = "",
  [string]$OutDir = "",
  [int]$DebugPort = 9223,
  [switch]$MonitorBoundary,
  [int]$TimeoutSec = 90,
  [string]$InstalledExe = "$env:LOCALAPPDATA\AgentPet\agentpet.exe"
)

$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$windowsDir = Split-Path -Parent $scriptDir
$cdpJs = Join-Path $scriptDir "native-qa-cdp.mjs"
$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
if (-not $OutDir) {
  $OutDir = Join-Path $env:TEMP "opencode\native-qa-$stamp"
}
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$logPath = Join-Path $OutDir "native-qa.log"
$reportPath = Join-Path $OutDir "report.json"
$realAppData = [Environment]::GetFolderPath("ApplicationData")
$realLocalAppData = [Environment]::GetFolderPath("LocalApplicationData")
$realProfile = $env:USERPROFILE
$realConfig = Join-Path $realAppData "AgentPet"
$backupDir = Join-Path $OutDir "user-config-backup"
$isoRoot = Join-Path $OutDir "isolated-profile"
$isoCfg = Join-Path $isoRoot "AgentPet"
$hookPort = 47628
$installedPath = [IO.Path]::GetFullPath($InstalledExe)

function Log([string]$msg) {
  $line = "[{0}] {1}" -f (Get-Date -Format "HH:mm:ss.fff"), $msg
  Add-Content -Path $logPath -Value $line
  Write-Host $line
}

Add-Type -AssemblyName System.Drawing | Out-Null
Add-Type -AssemblyName System.Windows.Forms | Out-Null
Add-Type -ReferencedAssemblies @("System.Drawing", "System.Windows.Forms", "System.Runtime.InteropServices") -TypeDefinition @"
using System;
using System.IO;
using System.Text;
using System.Threading;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Windows.Forms;
using System.Drawing;

public struct RECT { public int Left; public int Top; public int Right; public int Bottom; }
public struct POINT { public int X; public int Y; }
public struct MONITORINFOEX {
  public int cbSize;
  public RECT rcMonitor;
  public RECT rcWork;
  public int dwFlags;
  [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)]
  public string szDevice;
}
public struct INPUTN {
  public uint type;
  public MOUSEINPUT mi;
}
public struct MOUSEINPUT {
  public int dx;
  public int dy;
  public uint mouseData;
  public uint dwFlags;
  public uint time;
  public IntPtr dwExtraInfo;
}

public class NativeQa {
  static Thread frameWatch;
  static volatile bool watchFrames;
  static int frameSamples, frameDriftX, frameDriftY, frameWidths;
  public static void StartFrameWatch(long hwnd) {
    RECT initial;
    if (!GetWindowRect(new IntPtr(hwnd), out initial)) throw new Exception("No initial frame");
    int center2 = initial.Left + initial.Right, bottom = initial.Bottom;
    frameSamples = frameDriftX = frameDriftY = frameWidths = 0;
    watchFrames = true;
    frameWatch = new Thread(() => {
      var widths = new HashSet<int>();
      while (watchFrames) {
        RECT r;
        if (GetWindowRect(new IntPtr(hwnd), out r)) {
          frameSamples++;
          frameDriftX = Math.Max(frameDriftX, Math.Abs(r.Left + r.Right - center2));
          frameDriftY = Math.Max(frameDriftY, Math.Abs(r.Bottom - bottom));
          widths.Add(r.Right - r.Left);
        }
        Thread.Sleep(1);
      }
      frameWidths = widths.Count;
    });
    frameWatch.IsBackground = true;
    frameWatch.Start();
  }
  public static int[] StopFrameWatch() {
    watchFrames = false;
    if (frameWatch != null) frameWatch.Join();
    return new int[] { frameSamples, frameDriftX, frameDriftY, frameWidths };
  }
  public delegate bool EnumMonProc(IntPtr hMon, IntPtr hdc, ref RECT lprc, IntPtr dwData);
  public delegate bool EnumWinProc(IntPtr hWnd, IntPtr lParam);

  [DllImport("shcore.dll")] public static extern int SetProcessDpiAwareness(int value);
  [DllImport("shcore.dll")] public static extern int GetDpiForMonitor(IntPtr hmon, int dpiType, out uint dpiX, out uint dpiY);
  [DllImport("user32.dll")] public static extern bool EnumDisplayMonitors(IntPtr hdc, IntPtr lprcClip, EnumMonProc lpfn, IntPtr dwData);
  [DllImport("user32.dll")] public static extern IntPtr MonitorFromPoint(POINT pt, uint flags);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWinProc lpEnumFunc, IntPtr lParam);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint pid);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr hWnd);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr hWnd, StringBuilder lpString, int nMaxCount);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT lpRect);
  [DllImport("user32.dll")] public static extern int GetWindowLong(IntPtr hWnd, int nIndex);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hWnd, IntPtr after, int X, int Y, int cx, int cy, uint uFlags);
  [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(POINT p);
  [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr hWnd, uint gaFlags);
  [DllImport("user32.dll")] public static extern int GetSystemMetrics(int nIndex);
  [DllImport("user32.dll")] public static extern void mouse_event(uint dwFlags, int dx, int dy, uint dwData, UIntPtr extra);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);

  public static void DpiAware() {
    try { SetProcessDpiAwareness(2); } catch {}
  }

  public static string MonitorsJson() {
    var list = new List<string>();
    foreach (Screen s in Screen.AllScreens) {
      POINT p; p.X = s.Bounds.Left + 1; p.Y = s.Bounds.Top + 1;
      IntPtr hMon = MonitorFromPoint(p, 2);
      uint dx = 96, dy = 96;
      try { GetDpiForMonitor(hMon, 0, out dx, out dy); } catch {}
      string dev = (s.DeviceName ?? "").Replace("\\", "\\\\").Replace("\"", "\\\"");
      list.Add(string.Format("{{\"device\":\"{0}\",\"left\":{1},\"top\":{2},\"right\":{3},\"bottom\":{4},\"workLeft\":{5},\"workTop\":{6},\"workRight\":{7},\"workBottom\":{8},\"dpiX\":{9},\"dpiY\":{10},\"primary\":{11}}}",
        dev,
        s.Bounds.Left, s.Bounds.Top, s.Bounds.Right, s.Bounds.Bottom,
        s.WorkingArea.Left, s.WorkingArea.Top, s.WorkingArea.Right, s.WorkingArea.Bottom,
        dx, dy, s.Primary ? "true" : "false"));
    }
    return "[" + string.Join(",", list.ToArray()) + "]";
  }

  public static string WindowsJson(uint pid) {
    var list = new List<string>();
    EnumWindows((h, l) => {
      uint wpid;
      GetWindowThreadProcessId(h, out wpid);
      if (wpid != pid) return true;
      var sb = new StringBuilder(512);
      GetWindowText(h, sb, sb.Capacity);
      RECT rc;
      GetWindowRect(h, out rc);
      int style = GetWindowLong(h, -16);
      int ex = GetWindowLong(h, -20);
      bool vis = IsWindowVisible(h);
      list.Add(string.Format("{{\"hwnd\":{0},\"title\":\"{1}\",\"left\":{2},\"top\":{3},\"right\":{4},\"bottom\":{5},\"visible\":{6},\"style\":{7},\"exStyle\":{8},\"caption\":{9}}}",
        h.ToInt64(),
        (sb.ToString() ?? "").Replace("\\", "\\\\").Replace("\"", "\\\""),
        rc.Left, rc.Top, rc.Right, rc.Bottom,
        vis ? "true" : "false",
        style, ex,
        ((style & 0x00C00000) != 0) ? "true" : "false"));
      return true;
    }, IntPtr.Zero);
    return "[" + string.Join(",", list.ToArray()) + "]";
  }

  public static bool MoveWindow(long hwnd, int x, int y, int w, int h) {
    return SetWindowPos(new IntPtr(hwnd), new IntPtr(-1), x, y, w, h, 0x0040);
  }

  public static long PointWindow(int x, int y) {
    POINT p; p.X = x; p.Y = y;
    return WindowFromPoint(p).ToInt64();
  }

  public static long RootWindow(long hwnd) {
    if (hwnd == 0) return 0;
    return GetAncestor(new IntPtr(hwnd), 2).ToInt64();
  }

  public static void MouseMove(int x, int y) {
    SetCursorPos(x, y);
  }

  public static void MouseDown() {
    mouse_event(0x0002, 0, 0, 0, UIntPtr.Zero);
  }

  public static void MouseUp() {
    mouse_event(0x0004, 0, 0, 0, UIntPtr.Zero);
  }

  public static void ClickAt(int x, int y) {
    MouseMove(x, y);
    Thread.Sleep(30);
    MouseDown();
    // A sprite press starts native dragging through asynchronous WebView IPC.
    // Releasing first can leave a delayed drag active during the next test.
    Thread.Sleep(250);
    MouseUp();
  }

  public static void Drag(int x0, int y0, int x1, int y1, int steps) {
    MouseMove(x0, y0);
    Thread.Sleep(40);
    MouseDown();
    Thread.Sleep(250);
    if (steps < 2) steps = 2;
    for (int i = 1; i <= steps; i++) {
      int x = x0 + (x1 - x0) * i / steps;
      int y = y0 + (y1 - y0) * i / steps;
      MouseMove(x, y);
      Thread.Sleep(20);
    }
    Thread.Sleep(40);
    MouseUp();
  }

  public static volatile int ClickCount = 0;
  public static long FormHwnd = 0;
  static Form _form;

  public static void StartClickCounter(int x, int y, int w, int h, string readyFile) {
    var t = new Thread(() => {
      _form = new Form();
      _form.Text = "native-qa-clickcounter";
      _form.StartPosition = FormStartPosition.Manual;
      _form.Location = new Point(x, y);
      _form.Size = new Size(w, h);
      _form.FormBorderStyle = FormBorderStyle.None;
      _form.TopMost = false;
      _form.BackColor = Color.Magenta;
      _form.ShowInTaskbar = false;
      var lbl = new Label();
      lbl.Dock = DockStyle.Fill;
      lbl.TextAlign = ContentAlignment.MiddleCenter;
      lbl.Font = new Font("Segoe UI", 28);
      lbl.Text = "0";
      lbl.BackColor = Color.Magenta;
      _form.Controls.Add(lbl);
      EventHandler inc = (s, e) => {
        ClickCount++;
        lbl.Text = ClickCount.ToString();
        try { File.WriteAllText(readyFile + ".count", ClickCount.ToString()); } catch {}
      };
      _form.Click += inc;
      lbl.Click += inc;
      _form.Load += (s, e) => {
        FormHwnd = _form.Handle.ToInt64();
        try { File.WriteAllText(readyFile, FormHwnd.ToString()); } catch {}
      };
      Application.Run(_form);
    });
    t.SetApartmentState(ApartmentState.STA);
    t.IsBackground = true;
    t.Start();
  }

  public static void StopClickCounter() {
    try {
      if (_form != null) {
        _form.BeginInvoke(new Action(() => { _form.Close(); Application.ExitThread(); }));
      }
    } catch {}
  }
}
"@ | Out-Null

[NativeQa]::DpiAware() | Out-Null

$checks = New-Object System.Collections.Generic.List[object]
$blockers = New-Object System.Collections.Generic.List[string]
function Add-Check([string]$name, [bool]$ok, [string]$detail, $data = $null) {
  $checks.Add(@{ name = $name; ok = $ok; detail = $detail; data = $data })
  $tag = if ($ok) { "PASS" } else { "FAIL" }
  Log "$tag $name :: $detail"
}

function Invoke-Cdp {
  $raw = & node $cdpJs @args 2>&1 | Out-String
  $raw = $raw.Trim()
  if (-not $raw) { throw "cdp empty output: $($args -join ' ')" }
  try { return $raw | ConvertFrom-Json }
  catch { throw "cdp json parse failed: $raw" }
}

$script:GeomExpr = @'
(() => {
  const box = (el) => {
    if (!el || el.hidden) return null;
    const r = el.getBoundingClientRect();
    if (r.width <= 0 && r.height <= 0) return null;
    return { left: r.left, top: r.top, right: r.right, bottom: r.bottom, width: r.width, height: r.height };
  };
  const c = document.getElementById("pet");
  const b = document.getElementById("bubble");
  const party = document.getElementById("party-badge");
  const root = document.getElementById("pet-root");
  const row = b && b.querySelector(".brow");
  const rowH = row ? row.getBoundingClientRect().height : 0;
  const btns = b ? [...b.querySelectorAll("button")].map((x) => (x.textContent || "").trim()) : [];
  return {
    dpr: devicePixelRatio || 1,
    innerW: innerWidth,
    innerH: innerHeight,
    canvas: box(c),
    sprite: (() => {
      if (!c || !c.width || !c.height) return null;
      try {
        const px = c.getContext('2d').getImageData(0, 0, c.width, c.height).data;
        const points = [];
        for (let y = 0; y < c.height; y++) for (let x = 0; x < c.width; x++) {
          if (px[(y * c.width + x) * 4 + 3] > 128) points.push([x, y]);
        }
        if (!points.length) return null;
        const p = points[Math.floor(points.length / 2)];
        return { u: (p[0] + .5) / c.width, v: (p[1] + .5) / c.height };
      } catch { return null; }
    })(),
    bubble: box(b),
    party: box(party),
    root: box(root),
    bubbleHidden: !!(b && b.hidden),
    bubbleText: (b && b.innerText) || "",
    rowHeight: rowH,
    rowCount: b ? b.querySelectorAll(".brow").length : 0,
    structure: {
      single: !!(b && b.querySelector(".single-line")),
      listRows: b ? b.querySelectorAll(".brow").length : 0,
      carousel: !!(b && b.querySelector(".car-row")),
      compact: !!(b && b.querySelector(".cmp-head")),
      hidden: !!(b && b.hidden),
      empty: !!(b && !b.hidden && b.childElementCount === 0),
      allow: btns.some((t) => t === "Allow"),
      deny: btns.some((t) => t === "Deny"),
    },
    tailShift: root ? getComputedStyle(root).getPropertyValue("--tail-shift") : "",
    bubbleShift: root ? getComputedStyle(root).getPropertyValue("--bubble-shift") : "",
    petOffset: root ? getComputedStyle(root).getPropertyValue("--pet-offset") : "",
    violators: (() => {
      const out = [];
      const named = [["canvas", c], ["bubble", b], ["party", party], ["root", root]];
      for (const [name, el] of named) {
        const r = box(el);
        if (!r) continue;
        if (r.left < -1 || r.top < -1 || r.right > innerWidth + 1 || r.bottom > innerHeight + 1) {
          out.push({ name, left: r.left, top: r.top, right: r.right, bottom: r.bottom });
        }
      }
      return out;
    })(),
  };
})()
'@

function Invoke-MainCdp([string]$expr) {
  Invoke-TargetCdp "main" $expr
}

function Invoke-TargetCdp([string]$needle, [string]$expr) {
  $tmp = Join-Path $OutDir ("cdp-" + [guid]::NewGuid().ToString("N") + ".js")
  [IO.File]::WriteAllText($tmp, $expr)
  try { Invoke-Cdp @("eval-file", "$DebugPort", $needle, $tmp) }
  finally { Remove-Item $tmp -Force -ErrorAction SilentlyContinue }
}

function Get-PetGeom([long]$hwnd, [string]$cdpNeedle = "main") {
  for ($snapshotTry = 0; $snapshotTry -lt 12; $snapshotTry++) {
  $rc = New-Object RECT
  $ok = [NativeQa]::GetWindowRect([IntPtr]$hwnd, [ref]$rc)
  $dom = $null
  if ($cdpOk) {
    $dom = Invoke-TargetCdp $cdpNeedle $script:GeomExpr
  }
  $v = $dom.value
  $dpr = 1.0
  if ($v -and $v.dpr) { $dpr = [double]$v.dpr }
  if ($dpr -le 0) { $dpr = 1 }
  $after = New-Object RECT
  [NativeQa]::GetWindowRect([IntPtr]$hwnd, [ref]$after) | Out-Null
  # Never combine an old HWND origin with a new DOM viewport. CDP evaluation
  # takes long enough for a pending bubble resize to land between these reads.
  $sameFrame = $rc.Left -eq $after.Left -and $rc.Top -eq $after.Top -and $rc.Right -eq $after.Right -and $rc.Bottom -eq $after.Bottom
  $sameViewport = -not $v -or ([Math]::Abs(($rc.Right-$rc.Left) - $v.innerW*$dpr) -le 2 -and [Math]::Abs(($rc.Bottom-$rc.Top) - $v.innerH*$dpr) -le 2)
  if ($sameFrame -and $sameViewport) { break }
  Start-Sleep -Milliseconds 40
  }
  if (-not $sameFrame -or -not $sameViewport) { throw "Unable to correlate native/DOM frame for $cdpNeedle hwnd=$hwnd" }
  $ax = $null; $ay = $null
  if ($v -and $v.canvas -and $v.canvas.width -gt 0) {
    $ax = $rc.Left + ($v.canvas.left + $v.canvas.width / 2.0) * $dpr
    $ay = $rc.Top + $v.canvas.bottom * $dpr
  }
  $clipOk = $false
  if ($v) {
    $n = 0
    if ($v.violators) { $n = @($v.violators).Count }
    $clipOk = ($n -eq 0)
  }
  return @{
    ok = [bool]$ok
    left = $rc.Left; top = $rc.Top; right = $rc.Right; bottom = $rc.Bottom
    w = $rc.Right - $rc.Left; h = $rc.Bottom - $rc.Top
    dpr = $dpr
    anchorX = $ax; anchorY = $ay
    clipOk = $clipOk
    violators = $(if ($v -and $v.violators) { $v.violators } else { @() })
    dom = $v
  }
}

function Resolve-CdpHwnd([string]$needle, $pets) {
  # A recently parked pet may be clamped between the native snapshot and CDP
  # query. Re-read both rather than matching against a stale HWND position.
  for ($attempt = 0; $attempt -lt 8; $attempt++) {
  $scr = Invoke-TargetCdp $needle "({ sx: window.screenX, sy: window.screenY, dpr: devicePixelRatio || 1, href: location.href })"
  if (-not $scr.value) { return @{ ok = $false; reason = "no cdp"; needle = $needle } }
  $dpr = 1.0
  if ($scr.value.dpr) { $dpr = [double]$scr.value.dpr }
  $sx = [double]$scr.value.sx
  $sy = [double]$scr.value.sy
  $cands = @(@{ x = $sx; y = $sy }, @{ x = $sx * $dpr; y = $sy * $dpr })
  $pets = @((Get-PetWindows ([uint32]$candPid)).pets)
  $hits = @()
  foreach ($pw in @($pets)) {
    $ok = $false
    foreach ($c in $cands) {
      if ([Math]::Abs($pw.left - $c.x) -le 24 -and [Math]::Abs($pw.top - $c.y) -le 24) { $ok = $true }
    }
    if ($ok) { $hits += $pw }
  }
  if ($hits.Count -ne 1) {
    Start-Sleep -Milliseconds 80
    continue
  }
  return @{ ok = $true; win = $hits[0]; href = $scr.value.href; sx = $sx; sy = $sy; dpr = $dpr; needle = $needle }
  }
  return @{ ok = $false; reason = "ambiguous $($hits.Count)"; href = $scr.value.href; sx = $sx; sy = $sy; dpr = $dpr; needle = $needle }
}

function Pick-ThroughPoint($gg) {
  $dpr = $gg.dpr
  if ($dpr -le 0) { $dpr = 1 }
  $union = $null
  if ($gg.dom) {
    $parts = @($gg.dom.canvas, $gg.dom.bubble, $gg.dom.party) | Where-Object { $_ }
    if ($parts.Count -gt 0) {
      $union = @{
        l = $gg.left + ([double]($parts | ForEach-Object { $_.left } | Measure-Object -Minimum).Minimum) * $dpr
        t = $gg.top + ([double]($parts | ForEach-Object { $_.top } | Measure-Object -Minimum).Minimum) * $dpr
        r = $gg.left + ([double]($parts | ForEach-Object { $_.right } | Measure-Object -Maximum).Maximum) * $dpr
        b = $gg.top + ([double]($parts | ForEach-Object { $_.bottom } | Measure-Object -Maximum).Maximum) * $dpr
      }
    }
  }
  $pts = @(
    @{ x = $gg.left + 1; y = $gg.top + 1 },
    @{ x = $gg.right - 2; y = $gg.top + 1 },
    @{ x = $gg.left + 1; y = $gg.bottom - 2 },
    @{ x = $gg.right - 2; y = $gg.bottom - 2 }
  )
  foreach ($p in $pts) {
    if ($p.x -lt $gg.left -or $p.x -ge $gg.right -or $p.y -lt $gg.top -or $p.y -ge $gg.bottom) { continue }
    $inside = $false
    if ($union) {
      $inside = $p.x -ge $union.l -and $p.x -le $union.r -and $p.y -ge $union.t -and $p.y -le $union.b
    }
    if (-not $inside) { return @{ x = [int]$p.x; y = [int]$p.y; union = $union } }
  }
  return $null
}

function Wait-UnderForm([int]$x, [int]$y, [long]$formHwnd, [int]$tries = 8) {
  [NativeQa]::MouseMove($x, $y)
  for ($i = 0; $i -lt $tries; $i++) {
    Start-Sleep -Milliseconds 30
    $wfp = [NativeQa]::PointWindow($x, $y)
    $root = [NativeQa]::RootWindow($wfp)
    if ($root -eq $formHwnd -or $wfp -eq $formHwnd) {
      return @{ ok = $true; wfp = $wfp; root = $root }
    }
  }
  $wfp = [NativeQa]::PointWindow($x, $y)
  return @{ ok = $false; wfp = $wfp; root = [NativeQa]::RootWindow($wfp) }
}

function Get-SpritePoint($gg) {
  if (-not $gg.dom.sprite) { throw "canvas has no readable opaque sprite pixel" }
  return @{
    x = [int]($gg.left + ($gg.dom.canvas.left + $gg.dom.sprite.u * $gg.dom.canvas.width) * $gg.dpr)
    y = [int]($gg.top + ($gg.dom.canvas.top + $gg.dom.sprite.v * $gg.dom.canvas.height) * $gg.dpr)
  }
}

function Get-ListenPid([int]$port) {
  $lines = netstat -ano | Select-String ":$port\s"
  foreach ($ln in $lines) {
    if ($ln.Line -match "LISTENING\s+(\d+)\s*$") { return [int]$Matches[1] }
  }
  return $null
}

function Get-AgentPetPids([string]$pathWanted) {
  Get-CimInstance Win32_Process -Filter "Name='agentpet.exe'" | Where-Object {
    $_.ExecutablePath -and ([IO.Path]::GetFullPath($_.ExecutablePath) -ieq $pathWanted)
  }
}

function Get-ConfigHashes([string]$dir) {
  $snap = [ordered]@{}
  if (-not (Test-Path $dir)) { return $snap }
  Get-ChildItem -Path $dir -Force | ForEach-Object {
    # Startup/shutdown append diagnostics when the installed app is restored.
    # This is a log, not persisted user configuration or care progress.
    if ($_.Name -eq "debug.log") { return }
    if ($_.PSIsContainer) {
      $snap[$_.Name] = "dir:" + (@(Get-ChildItem $_.FullName -Recurse -File -ErrorAction SilentlyContinue).Count)
    } else {
      $h = Get-FileHash -Path $_.FullName -Algorithm SHA256
      $snap[$_.Name] = $h.Hash + ":" + $_.Length
    }
  }
  return $snap
}

function Wait-NativeRect([long]$hwnd, [int]$x, [int]$y, [int]$tries = 20) {
  $rc = New-Object RECT
  for ($i = 0; $i -lt $tries; $i++) {
    if ([NativeQa]::GetWindowRect([IntPtr]$hwnd, [ref]$rc)) {
      if ([Math]::Abs($rc.Left - $x) -le 8 -and [Math]::Abs($rc.Top - $y) -le 8) {
        return @{ ok = $true; left = $rc.Left; top = $rc.Top; right = $rc.Right; bottom = $rc.Bottom }
      }
    }
    Start-Sleep -Milliseconds 50
  }
  return @{ ok = $false; left = $rc.Left; top = $rc.Top; right = $rc.Right; bottom = $rc.Bottom }
}

function Get-PetWindows([uint32]$ProcId) {
  $raw = [NativeQa]::WindowsJson($ProcId)
  $wins = @()
  if ($raw -and $raw -ne "[]") { $wins = $raw | ConvertFrom-Json }
  $pets = @($wins | Where-Object {
    $_.visible -and $_.title -eq "AgentPet" -and (($_.right - $_.left) -ge 80) -and (($_.right - $_.left) -lt 900) -and (($_.bottom - $_.top) -ge 80)
  })
  return @{ all = @($wins); pets = $pets }
}

function Save-Shot([long]$hwnd, [string]$name) {
  $path = Join-Path $OutDir "$name.png"
  $rc = New-Object RECT
  if (-not [NativeQa]::GetWindowRect([IntPtr]$hwnd, [ref]$rc)) { return $null }
  $w = [Math]::Max(1, $rc.Right - $rc.Left)
  $h = [Math]::Max(1, $rc.Bottom - $rc.Top)
  $bmp = New-Object Drawing.Bitmap $w, $h
  $g = [Drawing.Graphics]::FromImage($bmp)
  try {
    $g.CopyFromScreen($rc.Left, $rc.Top, 0, 0, $bmp.Size)
    $bmp.Save($path, [Drawing.Imaging.ImageFormat]::Png)
  } finally {
    $g.Dispose()
    $bmp.Dispose()
  }
  return @{ path = $path; left = $rc.Left; top = $rc.Top; width = $w; height = $h }
}

function Test-NonBlank([string]$pngPath) {
  if (-not (Test-Path $pngPath)) { return @{ ok = $false; reason = "missing" } }
  $bmp = [Drawing.Bitmap]::FromFile($pngPath)
  try {
    $w = $bmp.Width; $h = $bmp.Height
    $opaque = 0; $seen = New-Object "System.Collections.Generic.HashSet[int]"
    for ($i = 0; $i -lt 80; $i++) {
      $x = [int](($i * 17 + 3) % [Math]::Max(1, $w))
      $y = [int](($i * 29 + 5) % [Math]::Max(1, $h))
      $c = $bmp.GetPixel($x, $y)
      if ($c.A -gt 20 -and ($c.R -gt 12 -or $c.G -gt 12 -or $c.B -gt 12)) {
        $opaque++
        [void]$seen.Add(($c.R -shl 16) -bor ($c.G -shl 8) -bor $c.B)
      }
    }
    return @{ ok = ($opaque -ge 8 -and $seen.Count -ge 3); opaque = $opaque; colors = $seen.Count }
  } finally { $bmp.Dispose() }
}

function Post-Event($body) {
  $json = $body | ConvertTo-Json -Compress
  $bytes = [Text.Encoding]::UTF8.GetBytes($json)
  $req = [Net.HttpWebRequest]::Create("http://127.0.0.1:$hookPort/event")
  $req.Method = "POST"
  $req.ContentType = "application/json"
  $req.Timeout = 3000
  $req.ReadWriteTimeout = 3000
  $stream = $req.GetRequestStream()
  $stream.Write($bytes, 0, $bytes.Length)
  $stream.Close()
  $resp = $req.GetResponse()
  $reader = New-Object IO.StreamReader($resp.GetResponseStream())
  $text = $reader.ReadToEnd()
  $reader.Close(); $resp.Close()
  return $text
}

$candidateProc = $null
$stoppedInstalled = @()
$clickReady = Join-Path $OutDir "clickcounter.ready"
$configBefore = Get-ConfigHashes $realConfig
$report = [ordered]@{
  started = (Get-Date).ToString("o")
  outDir = $OutDir
  monitors = @()
  mixedDpi = $false
  installed = $null
  candidate = $null
  checks = @()
  blockers = @()
  restoredInstalled = $false
  configLeak = $false
}

try {
  Log "out=$OutDir"
  $monJson = [NativeQa]::MonitorsJson()
  $monitors = @($monJson | ConvertFrom-Json)
  $report.monitors = $monitors
  $dpiSet = @($monitors | ForEach-Object { $_.dpiX } | Sort-Object -Unique)
  $report.mixedDpi = ($monitors.Count -gt 1 -and $dpiSet.Count -gt 1)
  Log ("monitors={0} dpi=[{1}] mixedDpi={2}" -f $monitors.Count, ($dpiSet -join ","), $report.mixedDpi)
  if ($monitors.Count -lt 1) { $blockers.Add("no monitors enumerated") }
  if (-not $report.mixedDpi) {
    $blockers.Add("mixed-DPI not attached (monitors=$($monitors.Count) dpi=[$($dpiSet -join ',')]); not fabricated")
  }
  Add-Check "monitors-enumerated" ($monitors.Count -ge 1) ("count=$($monitors.Count) dpi=[$($dpiSet -join ',')]") $monitors

  if (-not $Exe) {
    $debugExe = Join-Path $windowsDir "src-tauri\target\debug\agentpet.exe"
    $relExe = Join-Path $windowsDir "src-tauri\target\release\agentpet.exe"
    if (Test-Path $debugExe) { $Exe = $debugExe }
    elseif (Test-Path $relExe) { $Exe = $relExe }
  }
  if (-not (Test-Path $Exe)) { throw "candidate exe missing: $Exe" }
  $Exe = [IO.Path]::GetFullPath($Exe)
  Log "exe=$Exe"

  $isoCfg = Join-Path $isoRoot "AgentPet"
  $isoWeb = Join-Path $isoRoot "WebView2"
  New-Item -ItemType Directory -Force -Path $isoCfg, $isoWeb | Out-Null
  Set-Content -Path (Join-Path $isoCfg ".onboarded") -Value "1" -NoNewline
  Set-Content -Path (Join-Path $isoCfg "petvisible") -Value "1" -NoNewline
  Log ("qaProfile={0} (real AppData untouched)" -f $isoRoot)

  $installed = @(Get-AgentPetPids $installedPath)
  $report.installed = @($installed | ForEach-Object { @{ pid = $_.ProcessId; path = $_.ExecutablePath } })
  $listenBefore = Get-ListenPid $hookPort
  Log ("installed pids={0} hookPid={1}" -f (($installed | ForEach-Object { $_.ProcessId }) -join ","), $listenBefore)

  $needStop = $false
  if ($installed.Count -gt 0) { $needStop = $true }
  if ($listenBefore) { $needStop = $true }
  if ($needStop) {
    Log "RISK: stopping installed AgentPet at $installedPath so candidate can bind single-instance + :$hookPort; will restore in finally"
    foreach ($p in $installed) {
      Stop-Process -Id $p.ProcessId -Force -ErrorAction SilentlyContinue
      $stoppedInstalled += @{ pid = $p.ProcessId; path = $p.ExecutablePath }
    }
    $deadline = (Get-Date).AddSeconds(8)
    do {
      Start-Sleep -Milliseconds 250
      $left = @(Get-AgentPetPids $installedPath)
      $lp = Get-ListenPid $hookPort
    } while ((($left.Count -gt 0) -or $lp) -and (Get-Date) -lt $deadline)
    if (Get-ListenPid $hookPort) { throw "hook port $hookPort still held after stopping installed app" }
  }

  $usedPort = $null
  for ($p = [Math]::Max(9223, $DebugPort); $p -le 9229; $p++) {
    $owner = Get-ListenPid $p
    if (-not $owner) { $usedPort = $p; break }
  }
  if (-not $usedPort) { throw "no free CDP port in 9223-9229" }
  $DebugPort = $usedPort
  Log "cdpPort=$DebugPort"

  $psi = New-Object Diagnostics.ProcessStartInfo
  $psi.FileName = $Exe
  $psi.WorkingDirectory = Split-Path $Exe
  $psi.UseShellExecute = $false
  $psi.CreateNoWindow = $false
  $envMap = @{
    AGENTPET_QA_PROFILE = $isoRoot
    AGENTPET_CDP_PORT = "$DebugPort"
    WEBVIEW2_USER_DATA_FOLDER = $isoWeb
  }
  foreach ($k in $envMap.Keys) {
    $psi.EnvironmentVariables[$k] = $envMap[$k]
  }
  $candidateProc = [Diagnostics.Process]::Start($psi)
  if (-not $candidateProc) { throw "failed to start candidate" }
  $candPid = $candidateProc.Id
  $report.candidate = @{ pid = $candPid; path = $Exe; cdpPort = $DebugPort }
  Log "started candidate pid=$candPid"

  $ready = $false
  $deadline = (Get-Date).AddSeconds($TimeoutSec)
  $hookPid = $null
  while ((Get-Date) -lt $deadline) {
    if ($candidateProc.HasExited) { throw "candidate exited early code=$($candidateProc.ExitCode)" }
    $hookPid = Get-ListenPid $hookPort
    if ($hookPid -eq $candPid) {
      $cdpListen = Get-ListenPid $DebugPort
      $pwInfo = Get-PetWindows ([uint32]$candPid)
      $petHw = @($pwInfo.pets)
      Log ("hook ok; cdpListenPid={0} petWindows={1} allWindows={2} mainHwnd={3}" -f $cdpListen, $petHw.Count, @($pwInfo.all).Count, $candidateProc.MainWindowHandle)
      if (@($pwInfo.all).Count -gt 0 -and -not $script:dumpedWins) {
        Log ("windows raw={0}" -f ($pwInfo.all | ConvertTo-Json -Compress))
        $script:dumpedWins = $true
      }
      if ($cdpListen) {
        try {
          $w = Invoke-Cdp @("wait", "$DebugPort", "main", "3000")
          if ($w.ok) { $ready = $true; break }
        } catch {
          Log ("cdp wait: {0}" -f $_.Exception.Message)
        }
      }
      if ($petHw.Count -ge 1 -and $cdpListen) {
        Log "pet hwnd + cdp ready (cdpListen=$cdpListen)"
        $ready = $true
        break
      }
      if ($petHw.Count -ge 1 -and ((Get-Date) -gt $deadline.AddSeconds(-2))) {
        Log "pet hwnd ready without CDP (cdpListen=$cdpListen)"
        $ready = $true
        break
      }
    }
    Start-Sleep -Milliseconds 400
  }
  if (-not $ready) { throw "candidate not ready (hookPid=$hookPid candPid=$candPid)" }
  if ($hookPid -ne $candPid) { throw "hook port owned by pid=$hookPid not candidate $candPid; refusing synthetic POST" }
  Add-Check "candidate-hook-pid" ($hookPid -eq $candPid) "pid=$hookPid port=$hookPort"
  $cdpOk = $false
  $cdpNow = Get-ListenPid $DebugPort
  if ($cdpNow) {
    try {
      $pages = Invoke-Cdp @("list", "$DebugPort")
      Log ("cdp pages={0}" -f ($pages.targets | ConvertTo-Json -Compress))
      $cdpOk = ($pages.ok -eq $true)
      Add-Check "cdp-index" $cdpOk "targets=$($pages.targets.Count)" $pages.targets
    } catch {
      Add-Check "cdp-index" $false $_.Exception.Message $null
    }
  } else {
    Add-Check "cdp-index" $false "port $DebugPort not listening (wry sets AdditionalBrowserArguments so WEBVIEW2 env is ignored; qa.cdp.json overlay may not have applied)" $null
  }

  $dom = $null
  if ($cdpOk) {
    $dom = Invoke-Cdp @(
      "eval", "$DebugPort", "main",
      "(() => { const c=document.getElementById('pet'); const b=document.getElementById('bubble'); const r=document.getElementById('pet-root'); const cr=c?c.getBoundingClientRect():null; const br=b?b.getBoundingClientRect():null; const rr=r?r.getBoundingClientRect():null; return { dpr:devicePixelRatio, hidden:document.hidden, bubbleHidden: !!(b&&b.hidden), canvas:{w:c&&c.width,h:c&&c.height,cw:c&&c.clientWidth,ch:c&&c.clientHeight,rect:cr}, bubble:{text:(b&&b.innerText)||'', rect:br}, root:rr, title:document.title }; })()"
    )
    Log ("dom0={0}" -f ($dom | ConvertTo-Json -Compress))
  }

  $pw = Get-PetWindows ([uint32]$candPid)
  if ($pw.pets.Count -lt 1) { throw "no frameless pet window for pid $candPid" }
  $mainPet = $pw.pets | Sort-Object { $_.right - $_.left } -Descending | Select-Object -First 1
  $shot0 = Save-Shot ([int64]$mainPet.hwnd) "01-launch"
  $blank = Test-NonBlank $shot0.path
  Add-Check "visible-nonblank-canvas" ([bool]$blank.ok) ("opaque=$($blank.opaque) colors=$($blank.colors) $($shot0.path)") $shot0

  $token = "QA$stamp$([guid]::NewGuid().ToString('N').Substring(0,8))"
  $projA = "E:\native-qa\alpha"
  $projB = "E:\native-qa\beta"
  if (-not $cdpOk) { throw "CDP required for geometry assertions" }

  $cfg = Invoke-MainCdp @'
(async () => {
  try {
  const tokens = [
    { token: "dot", isVisible: true }, { token: "icon", isVisible: true },
    { token: "title", isVisible: true }, { token: "project", isVisible: true },
    { token: "separator", isVisible: true }, { token: "message", isVisible: true },
    { token: "stateLabel", isVisible: true }, { token: "elapsed", isVisible: true }
  ];
  localStorage.setItem("ap_multi", "1");
  localStorage.setItem("ap_idle", "0");
  localStorage.setItem("ap_bub_mode", "list");
  localStorage.setItem("ap_bub_grouping", "all");
  localStorage.setItem("ap_bub_max", "10");
  localStorage.setItem("ap_bub_filter", "all");
  localStorage.setItem("ap_bub_hidden", "[]");
  localStorage.setItem("ap_bub_tokens", JSON.stringify(tokens));
  localStorage.setItem("ap_font_size", "14");
  document.documentElement.style.setProperty("--bubble-font-size", "14px");
  if (window.__TAURI__ && window.__TAURI__.event) {
    await window.__TAURI__.event.emit("bubble-changed", null);
    await window.__TAURI__.event.emit("sessions-clear", null);
  }
  return {
    mode: localStorage.getItem("ap_bub_mode"),
    grouping: localStorage.getItem("ap_bub_grouping"),
    max: localStorage.getItem("ap_bub_max"),
    font: localStorage.getItem("ap_font_size"),
    idle: localStorage.getItem("ap_idle"),
    tokens: localStorage.getItem("ap_bub_tokens")
  };
  } catch (e) { return { ok: false, error: String(e && e.message ? e.message : e) }; }
})()
'@
  Log ("bubble cfg={0}" -f ($cfg | ConvertTo-Json -Depth 6 -Compress))
  $cfgOk = $cfg.value.mode -eq "list" -and $cfg.value.grouping -eq "all" -and $cfg.value.font -eq "14" -and $cfg.value.max -eq "10"
  Add-Check "bubble-config-keys" $cfgOk ("mode=$($cfg.value.mode) grouping=$($cfg.value.grouping) font=$($cfg.value.font) max=$($cfg.value.max)") $cfg.value
  Start-Sleep -Milliseconds 200
  $base = Get-PetGeom ([int64]$mainPet.hwnd) "main"
  if (-not $base.anchorX) { throw "baseline pet anchor missing (canvas rect)" }
  # Sample intermediate native frames, not just settled endpoints. A split
  # move/resize used to keep the final anchor but jump ~127px in between.
  [NativeQa]::MoveWindow([int64]$mainPet.hwnd, ([int]$work.workLeft + 500), ([int]$work.workTop + 300), ($base.right-$base.left), ($base.bottom-$base.top)) | Out-Null
  Start-Sleep -Milliseconds 1200
  [NativeQa]::StartFrameWatch([int64]$mainPet.hwnd)
  try {
    $resizeCycle = Invoke-MainCdp @'
(async () => {
  for (let i = 0; i < 8; i++) {
    for (const width of [180, 400, 240]) {
      await window.__TAURI__.core.invoke("resize_pet_window", {width, height: 200, petWidth: document.getElementById("pet").offsetWidth || 160});
      await new Promise(r => setTimeout(r, 30));
    }
  }
  return true;
})()
'@
  } finally { $motion = [NativeQa]::StopFrameWatch() }
  Add-Check "resize-transient-anchor" ($resizeCycle.value -eq $true -and $motion[0] -gt 40 -and $motion[1] -le 4 -and $motion[2] -le 2 -and $motion[3] -ge 3) "completed=$($resizeCycle.value) samples=$($motion[0]) maxX=$($motion[1]/2) maxY=$($motion[2]) widths=$($motion[3])" $motion
  Start-Sleep -Milliseconds 500
  [NativeQa]::StartFrameWatch([int64]$mainPet.hwnd)
  $completedCycles = 0
  try {
    for ($cycle = 0; $cycle -lt 6; $cycle++) {
      $transition = Invoke-MainCdp @'
(async () => {
  localStorage.setItem("ap_notify", "0");
  const payload = {agent: "opencode", session: "qa-motion-cycle", project: "motion-qa", title: "QA", ts: Date.now()};
  await window.__TAURI__.event.emit("agent-event", {...payload, state: "working", message: "Long working message before completing this task and returning to idle"});
  await new Promise(r => setTimeout(r, 500));
  await window.__TAURI__.event.emit("agent-event", {...payload, state: "done", message: "Done"});
  await new Promise(r => setTimeout(r, 500));
  await window.__TAURI__.event.emit("agent-end", "qa-motion-cycle");
  await new Promise(r => setTimeout(r, 500));
  return true;
})()
'@
      if ($transition.value -eq $true) { $completedCycles++ }
    }
    # Also watch the final 3s celebration expire into idle, not only the
    # settled frame after it. Subsequent five-row checks need a quiet baseline.
    Start-Sleep -Milliseconds 3500
  } finally { $doneMotion = [NativeQa]::StopFrameWatch() }
  Add-Check "working-done-idle-anchor" ($completedCycles -eq 6 -and $doneMotion[0] -gt 200 -and $doneMotion[1] -le 4 -and $doneMotion[2] -le 2 -and $doneMotion[3] -ge 2) "cycles=$completedCycles samples=$($doneMotion[0]) maxX=$($doneMotion[1]/2) maxY=$($doneMotion[2]) widths=$($doneMotion[3])" $doneMotion
  $base = Get-PetGeom ([int64]$mainPet.hwnd) "main"
  $ts0 = [Diagnostics.Stopwatch]::StartNew()
  $agents = @("jcode","claude","copilot","cursor","gemini")
  $i = 0
  foreach ($ag in $agents) {
    $i++
    Post-Event @{
      agent = $ag; event = "working"; session = "native-qa-$token-$i"; project = $projA
      title = "QA list row $i $token very-long-title-for-width"
      message = "QA list row $i $token lorem-wide-message-to-force-bubble-width"
      ts = [int64]([DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds())
    } | Out-Null
  }
  $grown = $null
  $grewH = $false; $grewW = $false; $hasToken = $false
  $deadlineGrow = (Get-Date).AddMilliseconds(2500)
  do {
    Start-Sleep -Milliseconds 120
    $grown = Get-PetGeom ([int64]$mainPet.hwnd) "main"
    $rowPx = 24.0 * $grown.dpr
    if ($grown.dom -and $grown.dom.rowHeight -gt 0) { $rowPx = [double]$grown.dom.rowHeight * $grown.dpr }
    $grewH = $grown.h -ge ($base.h + $rowPx)
    $grewW = $grown.w -gt ($base.w + 8)
    $hasToken = $grown.dom -and ($grown.dom.bubbleText -match [regex]::Escape($token))
  } while ((-not (($grewH -or $grewW) -and $hasToken)) -and (Get-Date) -lt $deadlineGrow)
  $growMs = $ts0.ElapsedMilliseconds
  $shotGrow = Save-Shot ([int64]$mainPet.hwnd) "02-bubble-grow"
  $anchorDx = [Math]::Abs([Math]::Round($grown.anchorX) - [Math]::Round($base.anchorX))
  $anchorDy = [Math]::Abs([Math]::Round($grown.anchorY) - [Math]::Round($base.anchorY))
  Add-Check "bubble-grow" (($grewH -or $grewW) -and $hasToken) ("base=$($base.w)x$($base.h) now=$($grown.w)x$($grown.h) rows=$($grown.dom.rowCount) token=$hasToken ${growMs}ms") $shotGrow
  Add-Check "grow-timing-bounded" ($growMs -lt 2500 -and (($grewH -or $grewW))) ("event-to-growth ${growMs}ms (bound 2500, not a perf claim)") $growMs
  Add-Check "content-in-viewport-grow" ([bool]$grown.clipOk) ("clipOk=$($grown.clipOk) inner=$($grown.dom.innerW)x$($grown.dom.innerH) rows=$($grown.dom.rowCount) violators=$($grown.violators | ConvertTo-Json -Compress)") $grown.dom
  Add-Check "stable-pet-anchor-grow" ($anchorDx -le 2 -and $anchorDy -le 2) ("canvas-bottom-center dX=$anchorDx dY=$anchorDy (tol 2px physical)") @{ baseX = $base.anchorX; baseY = $base.anchorY; nowX = $grown.anchorX; nowY = $grown.anchorY }

  $appr = Invoke-MainCdp @"
(async () => {
  if (window.__TAURI__ && window.__TAURI__.event) {
    await window.__TAURI__.event.emit("agent-approval", { id: "qa-appr-$token", session: "native-qa-$token-1", tool: "Bash", summary: "qa-approval" });
  }
  await new Promise((r) => setTimeout(r, 200));
  const b = document.getElementById("bubble");
  const btns = b ? [...b.querySelectorAll("button")].map((x) => (x.textContent || "").trim()) : [];
  return { allow: btns.includes("Allow"), deny: btns.includes("Deny"), btns };
})()
"@
  Add-Check "approval-display" ($appr.value.allow -eq $true -and $appr.value.deny -eq $true) ("btns=$($appr.value.btns -join ',')" ) $appr.value
  Invoke-MainCdp @"
(async () => {
  if (window.__TAURI__ && window.__TAURI__.event) {
    await window.__TAURI__.event.emit("agent-approval-resolved", { id: "qa-appr-$token", session: "native-qa-$token-1" });
  }
  return { ok: true };
})()
"@ | Out-Null

  foreach ($mode in @("list","carousel","compact")) {
    $modeR = Invoke-MainCdp @"
(async () => {
  localStorage.setItem("ap_bub_mode", "$mode");
  if (window.__TAURI__ && window.__TAURI__.event) await window.__TAURI__.event.emit("bubble-changed", null);
  await new Promise((r) => setTimeout(r, 120));
  const b = document.getElementById("bubble");
  return {
    mode: localStorage.getItem("ap_bub_mode"),
    hidden: !!(b && b.hidden),
    empty: !!(b && !b.hidden && b.childElementCount === 0),
    listRows: b ? b.querySelectorAll(".brow").length : 0,
    carousel: !!(b && b.querySelector(".car-row")),
    compact: !!(b && b.querySelector(".cmp-head")),
    single: !!(b && b.querySelector(".single-line")),
  };
})()
"@
    $st = $modeR.value
    $okMode = $false
    if ($mode -eq "list") { $okMode = (-not $st.hidden) -and (-not $st.empty) -and ($st.listRows -ge 1 -or $st.single) }
    elseif ($mode -eq "carousel") { $okMode = (-not $st.hidden) -and (-not $st.empty) -and ($st.carousel -or $st.single) }
    else { $okMode = (-not $st.hidden) -and (-not $st.empty) -and ($st.compact -or $st.single) }
    Add-Check "bubble-mode-$mode" $okMode ("hidden=$($st.hidden) empty=$($st.empty) list=$($st.listRows) car=$($st.carousel) cmp=$($st.compact) single=$($st.single)") $st
  }
  $hideAll = Invoke-MainCdp @'
(async () => {
  localStorage.setItem("ap_bub_mode", "list");
  localStorage.setItem("ap_bub_hidden", JSON.stringify(["jcode","claude","copilot","cursor","gemini"]));
  if (window.__TAURI__ && window.__TAURI__.event) await window.__TAURI__.event.emit("bubble-changed", null);
  await new Promise((r) => setTimeout(r, 150));
  const b = document.getElementById("bubble");
  return {
    hidden: !!(b && b.hidden),
    empty: !!(b && !b.hidden && b.childElementCount === 0),
    single: !!(b && b.querySelector(".single-line")),
    carousel: !!(b && b.querySelector(".car-row")),
    compact: !!(b && b.querySelector(".cmp-head")),
    listRows: b ? b.querySelectorAll(".brow").length : 0,
    text: (b && b.innerText) || ""
  };
})()
'@
  $fb = $hideAll.value
  $fbOk = ($fb.single -eq $true -or $fb.hidden -eq $true) -and (-not $fb.empty) -and ($fb.listRows -eq 0) -and (-not $fb.carousel) -and (-not $fb.compact)
  Add-Check "filter-hide-all-fallback" $fbOk ("empty-structured-container forbidden; single=$($fb.single) hidden=$($fb.hidden) empty=$($fb.empty) rows=$($fb.listRows)") $fb
  Invoke-MainCdp @'
(async () => {
  localStorage.setItem("ap_bub_hidden", "[]");
  localStorage.setItem("ap_bub_mode", "list");
  if (window.__TAURI__ && window.__TAURI__.event) await window.__TAURI__.event.emit("bubble-changed", null);
  return { ok: true };
})()
'@ | Out-Null
  Start-Sleep -Milliseconds 150

  Invoke-MainCdp @'
(async () => {
  if (window.__TAURI__ && window.__TAURI__.event) await window.__TAURI__.event.emit("sessions-clear", null);
  return { ok: true };
})()
'@ | Out-Null
  $shrunk = $null
  $didShrink = $false
  $deadlineSh = (Get-Date).AddMilliseconds(1500)
  do {
    Start-Sleep -Milliseconds 80
    $shrunk = Get-PetGeom ([int64]$mainPet.hwnd) "main"
    if ($grewH) { $didShrink = $shrunk.h -le ($grown.h - 8) }
    elseif ($grewW) { $didShrink = $shrunk.w -le ($grown.w - 8) }
    else { $didShrink = $false }
  } while ((-not $didShrink) -and (Get-Date) -lt $deadlineSh)
  $shotShrink = Save-Shot ([int64]$mainPet.hwnd) "03-bubble-shrink"
  Add-Check "bubble-shrink" $didShrink ("grown=$($grown.w)x$($grown.h) now=$($shrunk.w)x$($shrunk.h) grewH=$grewH grewW=$grewW") $shotShrink
  $shDx = [Math]::Abs([Math]::Round($shrunk.anchorX) - [Math]::Round($grown.anchorX))
  $shDy = [Math]::Abs([Math]::Round($shrunk.anchorY) - [Math]::Round($grown.anchorY))
  Add-Check "stable-pet-anchor-shrink" ($shDx -le 2 -and $shDy -le 2) ("dX=$shDx dY=$shDy") @{ gx = $grown.anchorX; gy = $grown.anchorY; sx = $shrunk.anchorX; sy = $shrunk.anchorY }

  $work = $monitors | Where-Object { $_.primary } | Select-Object -First 1
  if (-not $work) { $work = $monitors[0] }
  $inWork = $shrunk.left -ge $work.workLeft -and $shrunk.top -ge $work.workTop -and $shrunk.right -le ($work.workRight + 2) -and $shrunk.bottom -le ($work.workBottom + 2)
  Add-Check "clamp-primary-work" $inWork ("rect=$($shrunk.left),$($shrunk.top)-$($shrunk.right),$($shrunk.bottom) work=$($work.workLeft),$($work.workTop)-$($work.workRight),$($work.workBottom)") $work

  Log "edge-tests begin"
  try {
  Invoke-MainCdp @'
(async () => {
  localStorage.setItem("ap_bub_mode", "list");
  localStorage.setItem("ap_bub_grouping", "all");
  if (window.__TAURI__ && window.__TAURI__.event) await window.__TAURI__.event.emit("bubble-changed", null);
  return { ok: true };
})()
'@ | Out-Null
  Post-Event @{
    agent = "jcode"; event = "working"; session = "native-qa-$token-wide"; project = $projA
    title = "wide-edge $token"; message = "wide-edge $token xxxxxxxxxxxxxxxxxxxxxxxxx"
    ts = [int64]([DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds())
  } | Out-Null
  Start-Sleep -Milliseconds 600
  $wide = Get-PetGeom ([int64]$mainPet.hwnd) "main"
  $ww = $wide.w; $wh = $wide.h
  if ($MonitorBoundary) {
    $edges = @(
      @{ name = "right"; x = $work.workRight - 40; y = [int](($work.workTop + $work.workBottom) / 2 - $wh / 2) },
      @{ name = "left"; x = $work.workLeft - ($ww - 40); y = [int](($work.workTop + $work.workBottom) / 2 - $wh / 2) },
      @{ name = "bottom"; x = [int](($work.workLeft + $work.workRight) / 2 - $ww / 2); y = $work.workBottom - 40 },
      @{ name = "top"; x = [int](($work.workLeft + $work.workRight) / 2 - $ww / 2); y = $work.workTop - 40 }
    )
    foreach ($e in $edges) {
      Log ("edge $($e.name) move")
      $cur = Get-PetGeom ([int64]$mainPet.hwnd) "main"
      $ew = $cur.w; $eh = $cur.h
      if ($ew -lt 80) { $ew = $ww; $eh = $wh }
      [NativeQa]::MoveWindow([int64]$mainPet.hwnd, [int]$e.x, [int]$e.y, $ew, $eh) | Out-Null
      Start-Sleep -Milliseconds 1600
      $eg = Get-PetGeom ([int64]$mainPet.hwnd) "main"
      $inside = $eg.left -ge ($work.workLeft - 2) -and $eg.top -ge ($work.workTop - 2) -and $eg.right -le ($work.workRight + 2) -and $eg.bottom -le ($work.workBottom + 2)
      $tailRaw = ""
      if ($eg.dom) { $tailRaw = [string]$eg.dom.tailShift }
      $tailOk = ($tailRaw -eq "") -or ($tailRaw -match "^-?\d")
      Add-Check "edge-$($e.name)" ($inside -and $eg.clipOk -and $tailOk) ("rect=$($eg.left),$($eg.top)-$($eg.right),$($eg.bottom) clip=$($eg.clipOk) tail=$($eg.dom.tailShift) shift=$($eg.dom.bubbleShift) off=$($eg.dom.petOffset) violators=$($eg.violators | ConvertTo-Json -Compress)") $eg
    }
    Save-Shot ([int64]$mainPet.hwnd) "04-boundary" | Out-Null
  }
  } catch {
    Add-Check "edge-tests" $false $_.Exception.Message $null
    Log ("edge-tests error {0}" -f $_.Exception.Message)
  }

  $pwNow = Get-PetWindows ([uint32]$candPid)
  $stable = $pwNow.pets | Where-Object { $_.hwnd -eq $mainPet.hwnd } | Select-Object -First 1
  if (-not $stable) { $stable = $pwNow.pets | Select-Object -First 1 }

  $formX = [int]$work.workLeft + 80
  $formY = [int]$work.workTop + 80
  [NativeQa]::StartClickCounter($formX, $formY, 520, 420, $clickReady)
  $deadline = (Get-Date).AddSeconds(5)
  while (-not (Test-Path $clickReady) -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 50 }
  $formHwnd = if (Test-Path $clickReady) { [int64](Get-Content $clickReady -Raw) } else { 0 }
  Add-Check "clickcounter-ready" ($formHwnd -ne 0) "hwnd=$formHwnd"
  # WinForms coordinates can be DPI-virtualized; place the backing window in
  # physical pixels before bringing the candidate above it.
  [NativeQa]::MoveWindow($formHwnd, $formX, $formY, 900, 900) | Out-Null
  [NativeQa]::SetWindowPos([IntPtr]$formHwnd, [IntPtr](-2), 0, 0, 0, 0, 0x0043) | Out-Null
  [NativeQa]::SetForegroundWindow([IntPtr]$formHwnd) | Out-Null

  $w = $stable.right - $stable.left; $h = $stable.bottom - $stable.top
  $placeX = $formX + 40; $placeY = $formY + 40
  [NativeQa]::MoveWindow([int64]$stable.hwnd, $placeX, $placeY, $w, $h) | Out-Null
  $placed = Wait-NativeRect ([int64]$stable.hwnd) $placeX $placeY 25
  $pwNow = Get-PetWindows ([uint32]$candPid)
  $petNow = $pwNow.pets | Where-Object { $_.hwnd -eq $stable.hwnd } | Select-Object -First 1
  if (-not $petNow) { $petNow = $pwNow.pets | Select-Object -First 1 }
  Add-Check "pet-over-form" ([bool]$placed.ok) ("hwnd=$($petNow.hwnd) rect=$($petNow.left),$($petNow.top) want=$placeX,$placeY") $placed
  $exTransparent = $petNow -and (($petNow.exStyle -band 0x20) -ne 0)
  Log ("pet exStyle={0} WS_EX_TRANSPARENT={1}" -f $petNow.exStyle, $exTransparent)

   $through = Pick-ThroughPoint (Get-PetGeom ([int64]$petNow.hwnd) "main")
   if (-not $through) { throw "main pet has no verified transparent point" }
   $throughX = [int]$through.x
   $throughY = [int]$through.y
  $spriteX = [int](($petNow.left + $petNow.right) / 2)
  $spriteY = [int](($petNow.top + $petNow.bottom) / 2)
  if ($cdpOk) {
    $hit = Invoke-Cdp @(
      "eval", "$DebugPort", "main",
      "(() => { const c=document.getElementById('pet'); const r=c.getBoundingClientRect(); return { left:r.left, top:r.top, width:r.width, height:r.height, dpr:devicePixelRatio }; })()"
    )
    if ($hit.value -and $hit.value.width -gt 0) {
      $dpr = [double]$hit.value.dpr
      if ($dpr -le 0) { $dpr = 1 }
      $spriteX = [int]($petNow.left + ($hit.value.left + $hit.value.width / 2) * $dpr)
      $spriteY = [int]($petNow.top + ($hit.value.top + $hit.value.height / 2) * $dpr)
      Log ("cdp sprite css=($($hit.value.left),$($hit.value.top),$($hit.value.width)x$($hit.value.height)) dpr=$dpr screen=$spriteX,$spriteY")
    }
  }
  $countFile = "$clickReady.count"
  $spritePoint = Get-SpritePoint (Get-PetGeom ([int64]$petNow.hwnd) "main")
  $spriteX = $spritePoint.x; $spriteY = $spritePoint.y
  if (Test-Path $countFile) { Remove-Item $countFile -Force }
   $underMain = Wait-UnderForm $throughX $throughY $formHwnd
   if (-not $underMain.ok) { throw "main transparent point is not over clickcounter: $($underMain.root)" }
   [NativeQa]::ClickAt($throughX, $throughY)
  Start-Sleep -Milliseconds 80
  $c1 = if (Test-Path $countFile) { [int](Get-Content $countFile -Raw) } else { [NativeQa]::ClickCount }
  $wf1 = [NativeQa]::PointWindow($throughX, $throughY)
   $overMain = Wait-UnderForm $spriteX $spriteY ([int64]$petNow.hwnd) 20
   if (-not $overMain.ok) { throw "opaque sprite point does not belong to main pet" }
   [NativeQa]::ClickAt($spriteX, $spriteY)
  Start-Sleep -Milliseconds 80
  $c2 = if (Test-Path $countFile) { [int](Get-Content $countFile -Raw) } else { [NativeQa]::ClickCount }
  $wf2 = [NativeQa]::PointWindow($spriteX, $spriteY)
  Add-Check "clickthrough-transparent" ($c1 -ge 1) ("count=$c1 wfp=$wf1 form=$formHwnd pet=$($petNow.hwnd) at $throughX,$throughY transparent=$exTransparent") @{ count = $c1; wfp = $wf1; form = $formHwnd; pet = $petNow.hwnd }
  Add-Check "sprite-captures-click" ($c2 -eq $c1) ("count $c1->$c2 wfp=$wf2 sprite=$spriteX,$spriteY (capture = clickcount unchanged)") @{ count = $c2; wfp = $wf2 }
  Save-Shot ([int64]$petNow.hwnd) "05-clickthrough" | Out-Null

  Start-Sleep -Milliseconds 150
  $pwPre = Get-PetWindows ([uint32]$candPid)
  $beforeDrag = $pwPre.pets | Where-Object { $_.hwnd -eq $petNow.hwnd } | Select-Object -First 1
  if (-not $beforeDrag) { $beforeDrag = $petNow }
  $dragToX = $spriteX + 90
  $dragToY = $spriteY - 40
  [NativeQa]::Drag($spriteX, $spriteY, $dragToX, $dragToY, 12)
  Start-Sleep -Milliseconds 600
  $pwDrag = Get-PetWindows ([uint32]$candPid)
  $afterDrag = $pwDrag.pets | Where-Object { $_.hwnd -eq $beforeDrag.hwnd } | Select-Object -First 1
  $moved = $afterDrag -and (([Math]::Abs($afterDrag.left - $beforeDrag.left) -ge 20) -or ([Math]::Abs($afterDrag.top - $beforeDrag.top) -ge 20))
  Add-Check "native-drag-sprite" $moved ("from $($beforeDrag.left),$($beforeDrag.top) to $($afterDrag.left),$($afterDrag.top)") $afterDrag
  Save-Shot ([int64]$afterDrag.hwnd) "06-drag" | Out-Null

  if ($cdpOk) {
  $splitExpr = @"
(async () => {
  const fnv = (s) => { let h = 0x811c9dc5; for (let i = 0; i < s.length; i++) { h ^= s.charCodeAt(i); h = Math.imul(h, 0x01000193); } return 'p' + (h >>> 0).toString(16).padStart(8, '0'); };
  const idA = fnv('$projA');
  const idB = fnv('$projB');
  localStorage.setItem('ap_split','1');
  const map = {};
  map[idA] = localStorage.getItem('ap_pet') || 'qa';
  map[idB] = localStorage.getItem('ap_pet') || 'qa';
  localStorage.setItem('ap_project_pets', JSON.stringify(map));
  if (window.__TAURI__ && window.__TAURI__.core) {
    await window.__TAURI__.core.invoke('sync_project_windows', { projects: [idA, idB] });
    return { invoked: true, ids: [idA, idB] };
  }
  return { invoked: false, ids: [idA, idB] };
})()
"@
  $split = Invoke-Cdp @("eval", "$DebugPort", "main", $splitExpr)
  Log ("split invoke={0}" -f ($split | ConvertTo-Json -Compress))
  $deadline = (Get-Date).AddSeconds(8)
  $splitPets = @()
  do {
    Start-Sleep -Milliseconds 400
    $splitPets = @((Get-PetWindows ([uint32]$candPid)).pets)
  } while ($splitPets.Count -lt 3 -and (Get-Date) -lt $deadline)
  Add-Check "split-windows" ($splitPets.Count -ge 3) ("petWindows=$($splitPets.Count) invoked=$($split.value.invoked)") $splitPets
  if ($splitPets.Count -ge 2) {
    $a = $splitPets[0]; $b = $splitPets[1]
    $aw = $a.right - $a.left; $ah = $a.bottom - $a.top
    $bw = $b.right - $b.left; $bh = $b.bottom - $b.top
    [NativeQa]::MoveWindow([int64]$a.hwnd, 200, 240, $aw, $ah) | Out-Null
    [NativeQa]::MoveWindow([int64]$b.hwnd, 700, 240, $bw, $bh) | Out-Null
    $null = Wait-NativeRect ([int64]$a.hwnd) 200 240 20
    $null = Wait-NativeRect ([int64]$b.hwnd) 700 240 20
    Start-Sleep -Milliseconds 1600
    $splitPets = @((Get-PetWindows ([uint32]$candPid)).pets)
    $a = $splitPets | Where-Object { $_.hwnd -eq $a.hwnd } | Select-Object -First 1
    $b = $splitPets | Where-Object { $_.hwnd -eq $b.hwnd } | Select-Object -First 1
    $bBefore = @{ l = $b.left; t = $b.top }
    $dragNeedle = $null
    foreach ($id in @($split.value.ids)) {
      $match = Resolve-CdpHwnd "project=$id" $splitPets
      if ($match.ok -and [int64]$match.win.hwnd -eq [int64]$a.hwnd) { $dragNeedle = "project=$id"; break }
    }
    if (-not $dragNeedle) { $dragNeedle = "main" }
    $dragGeom = Get-PetGeom ([int64]$a.hwnd) $dragNeedle
    $dragPoint = Get-SpritePoint $dragGeom
    $cx = $dragPoint.x; $cy = $dragPoint.y
    [NativeQa]::Drag($cx, $cy, $cx + 80, $cy + 40, 8)
    Start-Sleep -Milliseconds 500
    $afterSplit = @((Get-PetWindows ([uint32]$candPid)).pets)
    $a2 = $afterSplit | Where-Object { $_.hwnd -eq $a.hwnd } | Select-Object -First 1
    $b2 = $afterSplit | Where-Object { $_.hwnd -eq $b.hwnd } | Select-Object -First 1
    $aMoved = $a2 -and (([Math]::Abs($a2.left - $a.left) -ge 15) -or ([Math]::Abs($a2.top - $a.top) -ge 15))
    $bStable = $b2 -and ([Math]::Abs($b2.left - $bBefore.l) -le 16) -and ([Math]::Abs($b2.top - $bBefore.t) -le 16)
    Add-Check "split-independent-drag" ($aMoved -and $bStable) ("aMoved=$aMoved bStable=$bStable a=$($a2.left),$($a2.top) b=$($b2.left),$($b2.top)") @{ a = $a2; b = $b2 }
    Save-Shot ([int64]$a.hwnd) "07-split-a" | Out-Null
    Save-Shot ([int64]$b.hwnd) "07-split-b" | Out-Null
    $si = 0
    foreach ($id in @($split.value.ids)) {
      $si++
      $needle = "project=$id"
      $petsNow = @((Get-PetWindows ([uint32]$candPid)).pets)
      $resolved = Resolve-CdpHwnd $needle $petsNow
      if (-not $resolved.ok) {
        Add-Check "split-hit-$si" $false ("hwnd map failed $($resolved.reason) href=$($resolved.href) sx=$($resolved.sx),$($resolved.sy)") $resolved
        continue
      }
      $targetHwnd = [int64]$resolved.win.hwnd
       # Park fully inside the work area. Off-screen parking races the clamp
       # loop and a stale frontend resize can undo the intended placement.
       $parkX = [int]($work.workRight - 700)
      $parkI = 0
      foreach ($pw in $petsNow) {
        if ([int64]$pw.hwnd -eq $targetHwnd) { continue }
        $pwW = $pw.right - $pw.left; $pwH = $pw.bottom - $pw.top
         $parkY = [int]($work.workTop + 20 + $parkI * 400)
         if (-not [NativeQa]::MoveWindow([int64]$pw.hwnd, $parkX, $parkY, [int]$pwW, [int]$pwH)) { throw "Unable to park pet $($pw.hwnd)" }
         Log "park hwnd=$($pw.hwnd) at=$parkX,$parkY target=$targetHwnd"
        $parkI++
      }
       Start-Sleep -Milliseconds 1200
       $placeX = $formX + 48; $placeY = $formY + 48
      $sw = $resolved.win.right - $resolved.win.left; $sh = $resolved.win.bottom - $resolved.win.top
      [NativeQa]::MoveWindow($targetHwnd, $placeX, $placeY, [int]$sw, [int]$sh) | Out-Null
      $placed = Wait-NativeRect $targetHwnd $placeX $placeY 20
      $gg = Get-PetGeom $targetHwnd $needle
      $through = Pick-ThroughPoint $gg
      $spriteX = 0; $spriteY = 0
      if ($gg.dom -and $gg.dom.canvas -and $gg.dom.canvas.width -gt 0) {
        $spriteX = [int]($gg.left + ($gg.dom.canvas.left + $gg.dom.canvas.width / 2) * $gg.dpr)
        $spriteY = [int]($gg.top + ($gg.dom.canvas.top + $gg.dom.canvas.height / 2) * $gg.dpr)
      }
      $assoc = @{ href = $resolved.href; hwnd = $targetHwnd; needle = $needle; placed = $placed; through = $through; gg = @{ l = $gg.left; t = $gg.top; r = $gg.right; b = $gg.bottom; union = $through.union } }
      $spritePoint = Get-SpritePoint $gg
      $spriteX = $spritePoint.x; $spriteY = $spritePoint.y
      if (-not $placed.ok -or -not $through) {
        Add-Check "split-hit-$si" $false ("place/through failed href=$($resolved.href) hwnd=$targetHwnd") $assoc
        continue
      }
      $under = Wait-UnderForm ([int]$through.x) ([int]$through.y) $formHwnd
      if (-not $under.ok) {
        Add-Check "split-hit-$si" $false ("under form expected at $($through.x),$($through.y) wfp=$($under.wfp) root=$($under.root) form=$formHwnd href=$($resolved.href)") $assoc
        continue
      }
      $cf = "$clickReady.count"
      $c0 = if (Test-Path $cf) { [int](Get-Content $cf -Raw) } else { 0 }
      [NativeQa]::ClickAt([int]$through.x, [int]$through.y)
      Start-Sleep -Milliseconds 80
      $c1s = if (Test-Path $cf) { [int](Get-Content $cf -Raw) } else { $c0 }
       $gg = Get-PetGeom $targetHwnd $needle
       $spritePoint = Get-SpritePoint $gg
       $spriteX = $spritePoint.x; $spriteY = $spritePoint.y
       $overPet = Wait-UnderForm $spriteX $spriteY $targetHwnd 20
       if (-not $overPet.ok) {
         Add-Check "split-hit-$si" $false "opaque sprite point is not owned by target pet" $overPet
         continue
       }
      [NativeQa]::ClickAt($spriteX, $spriteY)
      Start-Sleep -Milliseconds 80
      $c2s = if (Test-Path $cf) { [int](Get-Content $cf -Raw) } else { $c1s }
      $throughOk = $c1s -gt $c0
      $capOk = $c2s -eq $c1s
      Add-Check "split-hit-$si" ($throughOk -and $capOk) ("href=$($resolved.href) hwnd=$targetHwnd through $c0->$c1s capture $c1s->$c2s pt=$($through.x),$($through.y) sprite=$spriteX,$spriteY") $assoc
    }
  }
  try {
    Invoke-Cdp @("eval", "$DebugPort", "main", "(async()=>{ localStorage.setItem('ap_split','0'); localStorage.removeItem('ap_project_pets'); if(window.__TAURI__?.core) await window.__TAURI__.core.invoke('sync_project_windows',{projects:[]}); return {ok:true}; })()") | Out-Null
  } catch {}
  } else {
    Add-Check "split-windows" $false "skipped: no CDP" $null
  }

  if ($cdpOk) {
  $hide = Invoke-Cdp @("eval", "$DebugPort", "main", "(async()=>{ if(window.__TAURI__?.core){ await window.__TAURI__.core.invoke('set_pet_visible',{visible:false}); return {ok:true}; } return {ok:false}; })()")
  Start-Sleep -Milliseconds 400
  $hiddenWins = @((Get-PetWindows ([uint32]$candPid)).pets | Where-Object { $_.visible })
  $visCheck = Invoke-Cdp @("eval", "$DebugPort", "main", "({ hidden: document.hidden })")
  Add-Check "hide-timers" ($hide.value.ok -eq $true) ("invoke=$($hide.value.ok) visiblePets=$($hiddenWins.Count) document.hidden=$($visCheck.value.hidden)") $visCheck
  $show = Invoke-Cdp @("eval", "$DebugPort", "main", "(async()=>{ if(window.__TAURI__?.core){ await window.__TAURI__.core.invoke('set_pet_visible',{visible:true}); return {ok:true}; } return {ok:false}; })()")
  Start-Sleep -Milliseconds 400
  $shownWins = @((Get-PetWindows ([uint32]$candPid)).pets | Where-Object { $_.visible })
  Add-Check "show-timers" ($show.value.ok -eq $true -and $shownWins.Count -ge 1) ("visiblePets=$($shownWins.Count)") $shownWins
  } else {
    $hideHw = [int64]$mainPet.hwnd
    [NativeQa]::ShowWindow([IntPtr]$hideHw, 0) | Out-Null
    Start-Sleep -Milliseconds 400
    $hiddenWins = @((Get-PetWindows ([uint32]$candPid)).pets | Where-Object { $_.visible })
    Add-Check "hide-timers" ($hiddenWins.Count -eq 0) "ShowWindow SW_HIDE visiblePets=$($hiddenWins.Count)" $hiddenWins
    [NativeQa]::ShowWindow([IntPtr]$hideHw, 5) | Out-Null
    Start-Sleep -Milliseconds 400
    $shownWins = @((Get-PetWindows ([uint32]$candPid)).pets | Where-Object { $_.visible })
    Add-Check "show-timers" ($shownWins.Count -ge 1) "ShowWindow SW_SHOW visiblePets=$($shownWins.Count)" $shownWins
  }
  Save-Shot ([int64]$mainPet.hwnd) "08-show" | Out-Null

} catch {
  Log "ERROR $($_.Exception.Message)"
  Add-Check "harness" $false $_.Exception.Message $null
  $blockers.Add($_.Exception.Message)
} finally {
  try { [NativeQa]::StopClickCounter() } catch {}
  if ($candidateProc -and -not $candidateProc.HasExited) {
    Log "stopping candidate pid=$($candidateProc.Id)"
    try { $candidateProc.Kill() } catch {}
    try { $candidateProc.WaitForExit(5000) | Out-Null } catch {}
  }
  if ($Exe) {
  Get-CimInstance Win32_Process -Filter "Name='agentpet.exe'" | Where-Object {
    $_.ExecutablePath -and ([IO.Path]::GetFullPath($_.ExecutablePath) -ieq $Exe)
  } | ForEach-Object {
    Log "killing leftover candidate pid=$($_.ProcessId)"
    Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue
  }
  }
  $deadline = (Get-Date).AddSeconds(5)
  do {
    Start-Sleep -Milliseconds 200
    $lp = Get-ListenPid $hookPort
  } while ($lp -and (Get-Date) -lt $deadline)

  if ($stoppedInstalled.Count -gt 0 -and (Test-Path $installedPath)) {
    Log "restoring installed $installedPath"
    Start-Process -FilePath $installedPath | Out-Null
    $report.restoredInstalled = $true
  }

  $configAfter = Get-ConfigHashes $realConfig
  $report.configBefore = $configBefore
  $report.configAfter = $configAfter
  $leaked = @()
  $names = @($configBefore.Keys + $configAfter.Keys | Select-Object -Unique)
  foreach ($name in $names) {
    if ("$($configBefore[$name])" -ne "$($configAfter[$name])") { $leaked += $name }
  }
  if ($leaked.Count -gt 0) {
    $report.configLeak = $true
    Log ("CONFIG HASH CHANGE (real profile, not restored) {0}" -f ($leaked -join ','))
    Add-Check "real-config-untouched" $false ("changed=$($leaked -join ',')" ) @{ before = $configBefore; after = $configAfter }
  } else {
    Add-Check "real-config-untouched" $true "hashes unchanged" $configAfter
  }
  $qaLog = Join-Path $isoCfg "debug.log"
  if (Test-Path $qaLog) {
    Copy-Item $qaLog (Join-Path $OutDir "candidate-debug.log") -Force
  }
  $report.checks = $checks
  $report.blockers = $blockers
  $report.finished = (Get-Date).ToString("o")
  $report.pass = -not (@($checks | Where-Object { -not $_.ok }).Count)
  ($report | ConvertTo-Json -Depth 8) | Set-Content -Path $reportPath -Encoding UTF8
  Log "report $reportPath pass=$($report.pass)"
}

Write-Host "REPORT $reportPath"
if (-not $report.pass) { exit 1 }
exit 0
