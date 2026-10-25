# Records the densa.dev videos again from the real apps, in both themes, and writes them with
# their still frames into the site's assets/video folder. Build the apps first:
#   densa:             cargo build --release -p densa-widgets --example monitor
#                      cargo build --release -p trading-demo
#   densa-connectors:  cargo build --release -p file-viewer
#   densa-studio:      cargo build --release
# Needs ffmpeg on the PATH. The repositories are expected side by side; pass -Dev otherwise.
# -Only studio-live,studio-join records just those (only their apps need building).
param(
  [string]$Dev = (Resolve-Path "$PSScriptRoot\..\..").Path,
  [string]$Site = "$Dev\densa.dev",
  [string]$Work = "$env:TEMP\densa-site-videos",
  [string[]]$Only = @()
)
$ErrorActionPreference = "Stop"
$record = "$PSScriptRoot\record_window.ps1"
# Given through -File, the names arrive as one text: "a,b".
$Only = @($Only | ForEach-Object { $_ -split "," } | Where-Object { $_ })
$known = "studio-live", "studio-join", "monitor", "files", "trading"
$unknown = @($Only | Where-Object { $known -notcontains $_ })
if ($unknown.Count) { throw "no recording named $($unknown -join ', '); the recordings are $($known -join ', ')" }
function Wanted($name) { $Only.Count -eq 0 -or $Only -contains $name }
New-Item -ItemType Directory -Force $Work, "$Site\assets\video" | Out-Null

# Encodes a recording into the site, with its still frame at `$at` seconds.
function Publish($from, $name, $crf, $at, $filter = "") {
  $video = "$Site\assets\video\$name.mp4"
  $filters = if ($filter) { @("-vf", $filter) } else { @() }
  ffmpeg -v error -y -i $from @filters -an -c:v libx264 -preset slow -crf $crf -pix_fmt yuv420p -movflags +faststart $video
  ffmpeg -v error -y -ss $at -i $video -frames:v 1 -c:v libwebp -quality 80 "$Site\assets\video\$name.webp"
}

$studioQuery = '--query "SELECT date_trunc(''second'', time) AS second, avg(celsius) AS celsius, count(*) AS readings FROM recent GROUP BY 1 ORDER BY 1 DESC"'
# Sensors above their own site's alarm temperature, per region: the readings stream joined to
# sites.csv, counted by a live query and charted as bars.
$joinQuery = '--query "SELECT region, sum(CASE WHEN celsius > alarm_celsius THEN 1 ELSE 0 END) AS above_alarm, avg(celsius) AS celsius FROM readings GROUP BY 1 ORDER BY 1"'
# Studio is recorded a minute in, when its charts of the last minute are full.
$studioWait = 62000
# The sample's temperatures rise and fall over each minute of the clock, warmest at :15:
# started at :03, a clip a minute in records the sensors passing their alarms.
function AtSecond($second) { while ((Get-Date).Second -ne $second) { Start-Sleep -Milliseconds 100 } }
# Scroll a million rows, sort by the ms column twice (its title at 598, 53), scroll again.
$filesScript = "300 move 500 400; 800 wheel 500 400 -20; 1300 wheel 500 400 -40; 1800 wheel 500 400 -80; 2300 wheel 500 400 -160; 3400 click 598 53; 5200 click 598 53; 6600 wheel 500 400 -20; 7000 wheel 500 400 -20"

foreach ($theme in "dark", "light") {
  $light = if ($theme -eq "light") { "--light" } else { "" }
  if (Wanted "studio-live") {
    & $record -Exe "$Dev\densa-studio\target\release\densa-studio.exe" -Out "$Work\studio-live-$theme.mp4" `
      -ArgLine "--sample --fresh --live --seconds 90 $light $studioQuery" -WaitMs $studioWait -Seconds 10
    Publish "$Work\studio-live-$theme.mp4" "studio-live-$theme" 26 9
  }
  if (Wanted "studio-join") {
    AtSecond 3
    & $record -Exe "$Dev\densa-studio\target\release\densa-studio.exe" -Out "$Work\studio-join-$theme.mp4" `
      -ArgLine "--sample --fresh --live --seconds 90 $light $joinQuery" -WaitMs $studioWait -Seconds 10
    Publish "$Work\studio-join-$theme.mp4" "studio-join-$theme" 26 9
  }
  if (Wanted "monitor") {
    & $record -Exe "$Dev\rust-trading\target\release\examples\monitor.exe" -Out "$Work\monitor-$theme.mp4" `
      -ArgLine "--seconds 30 $light" -WaitMs 4000 -Seconds 10
    Publish "$Work\monitor-$theme.mp4" "monitor-$theme" 28 9
  }
  if (Wanted "files") {
    & $record -Exe "$Dev\densa-connectors\target\release\file-viewer.exe" -Out "$Work\files-$theme.mp4" `
      -ArgLine "--sample --seconds 30 $light" -WaitMs 1500 -Seconds 8 -Script $filesScript
    Publish "$Work\files-$theme.mp4" "files-$theme" 26 7
  }
}
# The trading grid has one theme; its top rows fit the page.
if (Wanted "trading") {
  & $record -Exe "$Dev\rust-trading\target\release\trading-demo.exe" -Out "$Work\trading-dark.mp4" `
    -ArgLine "--seconds 30" -WaitMs 3000 -Seconds 8
  Publish "$Work\trading-dark.mp4" "trading-dark" 27 7 "crop=498:520:0:0"
}
Get-ChildItem "$Site\assets\video" | Select-Object Name, Length
