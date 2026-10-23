# Records the client area of one program's window to an MP4, frame by frame with PrintWindow
# (that window's own pixels only: nothing behind it, nothing else on the screen).
# Frames are kept in real time: when capturing falls behind, the last frame is repeated.
param(
  [string]$Exe,
  [string]$Out,
  [string]$ArgLine = "",
  [int]$WaitMs = 3000,
  [double]$Seconds = 8,
  [int]$Fps = 30,
  [int]$Crf = 24,
  # Input to post to the window while recording, one step per line: "MS move X Y",
  # "MS click X Y", "MS wheel X Y NOTCHES" (client pixels; negative notches scroll down).
  [string]$Script = ""
)
Add-Type -ReferencedAssemblies System.Drawing @"
using System;
using System.Diagnostics;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
public static class Rec {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
  [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] static extern bool GetClientRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] static extern bool ClientToScreen(IntPtr h, ref POINT p);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
  [DllImport("user32.dll")] static extern bool GetCursorPos(out POINT p);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);

  static IntPtr Point(int x, int y) { return (IntPtr)((y << 16) | (x & 0xFFFF)); }

  // Posts one scripted step to the window: the real pointer never moves.
  static void Post(IntPtr h, string[] step) {
    int x = int.Parse(step[2]), y = int.Parse(step[3]);
    switch (step[1]) {
      case "move": PostMessage(h, 0x0200, IntPtr.Zero, Point(x, y)); break;
      case "click":
        PostMessage(h, 0x0200, IntPtr.Zero, Point(x, y));
        PostMessage(h, 0x0201, (IntPtr)1, Point(x, y));
        PostMessage(h, 0x0202, IntPtr.Zero, Point(x, y));
        break;
      case "wheel":
        PostMessage(h, 0x0200, IntPtr.Zero, Point(x, y));
        POINT at = new POINT(); at.X = x; at.Y = y; ClientToScreen(h, ref at);
        PostMessage(h, 0x020A, (IntPtr)((int.Parse(step[4]) * 120) << 16), Point(at.X, at.Y));
        break;
    }
  }

  // Whether the window is shown with a client area to record.
  public static bool Ready(IntPtr h) {
    RECT cr;
    return h != IntPtr.Zero && IsWindowVisible(h) && GetClientRect(h, out cr) && cr.R > 0 && cr.B > 0;
  }
  [DllImport("user32.dll")] static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);

  // Moves the window (ours) off the pointer, so it shows no hover: the pointer stays put.
  public static void AvoidCursor(IntPtr h) {
    POINT c; GetCursorPos(out c);
    RECT r; GetWindowRect(h, out r);
    if (c.X < r.L || c.X >= r.R || c.Y < r.T || c.Y >= r.B) return;
    int w = r.R - r.L;
    int x = c.X - w - 40 >= 0 ? c.X - w - 40 : c.X + 40;
    SetWindowPos(h, IntPtr.Zero, x, r.T, 0, 0, 0x0001 | 0x0004 | 0x0010);  // NOSIZE NOZORDER NOACTIVATE
  }

  public static string Record(IntPtr h, string ffmpeg, string output, double seconds, int fps, int crf, string script) {
    var steps = new System.Collections.Generic.List<string[]>();
    foreach (var line in script.Split(new[] { '\n', ';' })) {
      var step = line.Trim().Split(new[] { ' ' }, StringSplitOptions.RemoveEmptyEntries);
      if (step.Length >= 4) steps.Add(step);
    }
    int nextStep = 0;
    RECT wr; GetWindowRect(h, out wr);
    RECT cr; GetClientRect(h, out cr);
    POINT origin = new POINT(); ClientToScreen(h, ref origin);
    int w = (cr.R / 2) * 2, hgt = (cr.B / 2) * 2;  // even sizes for 4:2:0 video
    int ox = origin.X - wr.L, oy = origin.Y - wr.T;
    var info = new ProcessStartInfo(ffmpeg,
      "-y -loglevel error -f rawvideo -pix_fmt bgra -s " + w + "x" + hgt + " -r " + fps +
      " -i - -c:v libx264 -preset slow -crf " + crf + " -pix_fmt yuv420p -movflags +faststart \"" + output + "\"");
    info.UseShellExecute = false; info.RedirectStandardInput = true;
    // The redirected input's writer starts with the console encoding's byte-order mark,
    // which would shift every pixel by three bytes: an encoding without one.
    Console.InputEncoding = new System.Text.UTF8Encoding(false);
    var proc = Process.Start(info);
    var input = proc.StandardInput.BaseStream;
    var whole = new Bitmap(wr.R - wr.L, wr.B - wr.T, PixelFormat.Format32bppArgb);
    var row = new byte[w * 4];
    var frame = new byte[w * hgt * 4];
    int total = (int)(seconds * fps), written = 0, captured = 0;
    var clock = Stopwatch.StartNew();
    while (written < total) {
      while (nextStep < steps.Count && int.Parse(steps[nextStep][0]) <= clock.ElapsedMilliseconds) {
        Post(h, steps[nextStep++]);
      }
      using (var g = Graphics.FromImage(whole)) {
        IntPtr hdc = g.GetHdc();
        PrintWindow(h, hdc, 2);  // PW_RENDERFULLCONTENT
        g.ReleaseHdc(hdc);
      }
      var data = whole.LockBits(new Rectangle(ox, oy, w, hgt), ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
      for (int y = 0; y < hgt; y++) {
        Marshal.Copy(data.Scan0 + y * data.Stride, frame, y * w * 4, w * 4);
      }
      whole.UnlockBits(data);
      captured++;
      // Write this frame for every frame slot that has passed, at least once.
      int due = Math.Min(total, (int)(clock.Elapsed.TotalSeconds * fps) + 1);
      do { input.Write(frame, 0, frame.Length); written++; } while (written < due);
      double next = (double)written / fps;
      int sleep = (int)((next - clock.Elapsed.TotalSeconds) * 1000);
      if (sleep > 0) System.Threading.Thread.Sleep(sleep);
    }
    input.Close();
    proc.WaitForExit();
    return w + "x" + hgt + ", " + written + " frames from " + captured + " captures";
  }
}
"@
[Rec]::SetProcessDPIAware() | Out-Null
$p = Start-Process -FilePath $Exe -ArgumentList $ArgLine -PassThru `
  -RedirectStandardOutput "$Out.stdout.txt" -RedirectStandardError "$Out.stderr.txt"
# Off the pointer as soon as the window shows, before anything in it can be hovered.
$started = Get-Date
$h = [IntPtr]::Zero
while (-not [Rec]::Ready($h) -and ((Get-Date) - $started).TotalSeconds -lt 30) {
  Start-Sleep -Milliseconds 50
  $p.Refresh()
  $h = $p.MainWindowHandle
}
if (-not [Rec]::Ready($h)) { throw "no window" }
[Rec]::AvoidCursor($h)
$left = $WaitMs - ((Get-Date) - $started).TotalMilliseconds
if ($left -gt 0) { Start-Sleep -Milliseconds $left }
$p.Refresh()
$h = $p.MainWindowHandle
if (-not [Rec]::Ready($h)) { throw "the window went away" }
$ffmpeg = (Get-Command ffmpeg).Source
[Rec]::Record($h, $ffmpeg, $Out, $Seconds, $Fps, $Crf, $Script)
if (-not $p.HasExited) { $p.Kill() }
