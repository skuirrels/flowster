<#
.SYNOPSIS
  Visio oracle (SPEC §22.3): builds Flowster's own test drawings through Visio's
  public COM automation API and exports Visio's rendering of every page as the
  reference image.

.DESCRIPTION
  Requires a licensed Microsoft Visio on Windows. Shapes are drawn with the
  drawing primitives only (DrawRectangle, DrawOval, DrawPolyline, ...), so the
  drawings contain no Microsoft stencil content. Output goes to target/corpus
  (git-ignored): <name>.vsdx plus <name>-p<N>.png per page.

  pwsh tools/visio-oracle/make-corpus.ps1 [-OutDir target/corpus]
#>
param(
  [string]$OutDir = (Join-Path $PSScriptRoot "..\..\target\corpus")
)
$ErrorActionPreference = "Stop"
$OutDir = [System.IO.Path]::GetFullPath($OutDir)
New-Item -ItemType Directory -Force $OutDir | Out-Null

# Visio constants (documented in the public Visio object model reference).
$visSelTypeEmpty = 0
$visSelect = 2
$visLayerVisible = 4
$visSectionObject = 1
$visRowLayerMem = 6
$visLayerMember = 0
$visCharacterStyle = 2
$visCharacterColor = 1
$visCharacterSize = 7

function Set-Formula($shape, [string]$cell, [string]$formula) {
  $shape.CellsU($cell).FormulaU = $formula
}

function Set-CharProp($chars, [int]$prop, $value) {
  [void]$chars.GetType().InvokeMember("CharProps", [Reflection.BindingFlags]::SetProperty, $null, $chars, @($prop, $value))
}

function New-TestEmf([string]$path) {
  # A 400 x 300 px metafile of plain GDI records (EmfOnly: no EMF+ records).
  Add-Type -AssemblyName System.Drawing
  if (Test-Path $path) { Remove-Item $path }
  $ref = [System.Drawing.Graphics]::FromHwnd([IntPtr]::Zero)
  $hdc = $ref.GetHdc()
  $frame = New-Object System.Drawing.RectangleF 0, 0, 400, 300
  $mf = New-Object System.Drawing.Imaging.Metafile $path, $hdc, $frame, ([System.Drawing.Imaging.MetafileFrameUnit]::Pixel), ([System.Drawing.Imaging.EmfType]::EmfOnly)
  $ref.ReleaseHdc($hdc)
  $ref.Dispose()
  $g = [System.Drawing.Graphics]::FromImage($mf)
  $black = New-Object System.Drawing.Pen ([System.Drawing.Color]::Black), 3
  $g.FillRectangle((New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(68, 114, 196))), 10, 10, 150, 90)
  $g.DrawRectangle($black, 10, 10, 150, 90)
  $g.FillEllipse((New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(192, 0, 0))), 190, 15, 120, 80)
  $green = New-Object System.Drawing.Pen ([System.Drawing.Color]::FromArgb(0, 150, 60)), 5
  $g.DrawLines($green, [System.Drawing.Point[]]@((New-Object System.Drawing.Point 20, 140), (New-Object System.Drawing.Point 80, 200), (New-Object System.Drawing.Point 140, 130), (New-Object System.Drawing.Point 200, 210)))
  $star = [System.Drawing.Point[]]@((New-Object System.Drawing.Point 300, 120), (New-Object System.Drawing.Point 320, 170), (New-Object System.Drawing.Point 375, 170), (New-Object System.Drawing.Point 330, 200), (New-Object System.Drawing.Point 345, 255), (New-Object System.Drawing.Point 300, 220), (New-Object System.Drawing.Point 255, 255), (New-Object System.Drawing.Point 270, 200), (New-Object System.Drawing.Point 225, 170), (New-Object System.Drawing.Point 280, 170))
  $g.FillPolygon((New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(255, 192, 0))), $star)
  $g.DrawPolygon((New-Object System.Drawing.Pen ([System.Drawing.Color]::Black), 1), $star)
  $g.DrawBezier((New-Object System.Drawing.Pen ([System.Drawing.Color]::FromArgb(112, 48, 160)), 4), 20, 280, 80, 220, 140, 300, 200, 240)
  $dash = New-Object System.Drawing.Pen ([System.Drawing.Color]::Black), 2
  $dash.DashStyle = [System.Drawing.Drawing2D.DashStyle]::Dash
  $g.DrawLine($dash, 210, 285, 390, 285)
  $font = New-Object System.Drawing.Font "Arial", 20, ([System.Drawing.FontStyle]::Regular), ([System.Drawing.GraphicsUnit]::Pixel)
  $g.DrawString("EMF text", $font, [System.Drawing.Brushes]::Black, 200, 100)
  $state = $g.Save()
  $g.TranslateTransform(120, 250)
  $g.RotateTransform(-30)
  $g.FillRectangle((New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(112, 173, 71))), -30, -12, 60, 24)
  $g.Restore($state)
  $bmp = New-Object System.Drawing.Bitmap 16, 16
  for ($y = 0; $y -lt 16; $y++) { for ($x = 0; $x -lt 16; $x++) {
    $c = if ((($x -shr 2) + ($y -shr 2)) % 2 -eq 0) { [System.Drawing.Color]::Black } else { [System.Drawing.Color]::White }
    $bmp.SetPixel($x, $y, $c)
  } }
  $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::NearestNeighbor
  $g.DrawImage($bmp, 345, 15, 48, 48)
  $g.Dispose()
  $mf.Dispose()
}

function New-TestEmfGdi([string]$path) {
  # Classic GDI records through P/Invoke: MM_ANISOTROPIC mapping, RECTANGLE,
  # ELLIPSE, ROUNDRECT, ARC/PIE/CHORD, MOVETO/LINETO, CREATEPEN, TEXTOUT.
  if (-not ("FlowsterGdi" -as [type])) {
    Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class FlowsterGdi {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [DllImport("gdi32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr CreateEnhMetaFileW(IntPtr hdc, string file, ref RECT frame, string desc);
  [DllImport("gdi32.dll")] public static extern IntPtr CloseEnhMetaFile(IntPtr hdc);
  [DllImport("gdi32.dll")] public static extern bool DeleteEnhMetaFile(IntPtr h);
  [DllImport("gdi32.dll")] public static extern int SetMapMode(IntPtr hdc, int mode);
  [DllImport("gdi32.dll")] public static extern bool SetWindowExtEx(IntPtr hdc, int x, int y, IntPtr old);
  [DllImport("gdi32.dll")] public static extern bool SetViewportExtEx(IntPtr hdc, int x, int y, IntPtr old);
  [DllImport("gdi32.dll")] public static extern int GetDeviceCaps(IntPtr hdc, int index);
  [DllImport("gdi32.dll")] public static extern IntPtr CreatePen(int style, int width, int color);
  [DllImport("gdi32.dll")] public static extern IntPtr CreateSolidBrush(int color);
  [DllImport("gdi32.dll")] public static extern IntPtr GetStockObject(int i);
  [DllImport("gdi32.dll")] public static extern IntPtr SelectObject(IntPtr hdc, IntPtr obj);
  [DllImport("gdi32.dll")] public static extern bool DeleteObject(IntPtr obj);
  [DllImport("gdi32.dll")] public static extern bool Rectangle(IntPtr hdc, int l, int t, int r, int b);
  [DllImport("gdi32.dll")] public static extern bool Ellipse(IntPtr hdc, int l, int t, int r, int b);
  [DllImport("gdi32.dll")] public static extern bool RoundRect(IntPtr hdc, int l, int t, int r, int b, int w, int h);
  [DllImport("gdi32.dll")] public static extern bool Arc(IntPtr hdc, int l, int t, int r, int b, int x1, int y1, int x2, int y2);
  [DllImport("gdi32.dll")] public static extern bool Pie(IntPtr hdc, int l, int t, int r, int b, int x1, int y1, int x2, int y2);
  [DllImport("gdi32.dll")] public static extern bool Chord(IntPtr hdc, int l, int t, int r, int b, int x1, int y1, int x2, int y2);
  [DllImport("gdi32.dll")] public static extern bool MoveToEx(IntPtr hdc, int x, int y, IntPtr old);
  [DllImport("gdi32.dll")] public static extern bool LineTo(IntPtr hdc, int x, int y);
  [DllImport("gdi32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr CreateFontW(int h, int w, int esc, int orient, int weight, int italic, int underline, int strike, int charset, int outPrec, int clipPrec, int quality, int pitch, string face);
  [DllImport("gdi32.dll")] public static extern int SetTextColor(IntPtr hdc, int color);
  [DllImport("gdi32.dll")] public static extern int SetBkMode(IntPtr hdc, int mode);
  [DllImport("gdi32.dll", CharSet = CharSet.Unicode)] public static extern bool TextOutW(IntPtr hdc, int x, int y, string s, int n);
  [DllImport("user32.dll")] public static extern IntPtr GetDC(IntPtr hwnd);
  [DllImport("user32.dll")] public static extern int ReleaseDC(IntPtr hwnd, IntPtr hdc);
}
"@
  }
  if (Test-Path $path) { Remove-Item $path }
  $G = [FlowsterGdi]
  $screen = $G::GetDC([IntPtr]::Zero)
  $pxPerMm = $G::GetDeviceCaps($screen, 8) / $G::GetDeviceCaps($screen, 4)
  # 100 mm x 75 mm picture with a 1000 x 750 logical grid.
  $frame = New-Object FlowsterGdi+RECT
  $frame.R = 10000; $frame.B = 7500
  $dc = $G::CreateEnhMetaFileW($screen, $path, [ref]$frame, "Flowster GDI test")
  [void]$G::ReleaseDC([IntPtr]::Zero, $screen)
  [void]$G::SetMapMode($dc, 8)
  [void]$G::SetWindowExtEx($dc, 1000, 750, [IntPtr]::Zero)
  [void]$G::SetViewportExtEx($dc, [int](100 * $pxPerMm), [int](75 * $pxPerMm), [IntPtr]::Zero)
  $rgb = { param($r, $g, $b) $r + 256 * $g + 65536 * $b }
  $pen = $G::CreatePen(0, 8, (& $rgb 0 0 0))
  $old = $G::SelectObject($dc, $pen)
  $brush = $G::CreateSolidBrush((& $rgb 91 155 213))
  [void]$G::SelectObject($dc, $brush)
  [void]$G::Rectangle($dc, 30, 30, 330, 230)
  $brush2 = $G::CreateSolidBrush((& $rgb 237 125 49))
  [void]$G::SelectObject($dc, $brush2)
  [void]$G::Ellipse($dc, 370, 30, 650, 230)
  [void]$G::RoundRect($dc, 690, 30, 970, 230, 80, 80)
  $null_brush = $G::GetStockObject(5)
  [void]$G::SelectObject($dc, $null_brush)
  [void]$G::Arc($dc, 30, 280, 330, 520, 330, 400, 180, 280)
  [void]$G::SelectObject($dc, $brush)
  [void]$G::Pie($dc, 370, 280, 650, 520, 650, 400, 510, 280)
  [void]$G::Chord($dc, 690, 280, 970, 520, 970, 400, 690, 400)
  $red = $G::CreatePen(0, 12, (& $rgb 192 0 0))
  [void]$G::SelectObject($dc, $red)
  [void]$G::MoveToEx($dc, 30, 720, [IntPtr]::Zero)
  [void]$G::LineTo($dc, 330, 580)
  [void]$G::LineTo($dc, 630, 720)
  $font = $G::CreateFontW(-60, 0, 0, 0, 700, 0, 0, 0, 0, 0, 0, 0, 0, "Arial")
  [void]$G::SelectObject($dc, $font)
  [void]$G::SetBkMode($dc, 1)
  [void]$G::SetTextColor($dc, (& $rgb 0 0 0))
  [void]$G::TextOutW($dc, 680, 600, "GDI text", 8)
  [void]$G::SelectObject($dc, $old)
  foreach ($o in $pen, $brush, $brush2, $red, $font) { [void]$G::DeleteObject($o) }
  [void]$G::DeleteEnhMetaFile($G::CloseEnhMetaFile($dc))
}

function Export-Doc($doc, [string]$name) {
  $path = Join-Path $OutDir "$name.vsdx"
  if (Test-Path $path) { Remove-Item $path }
  [void]$doc.SaveAs($path)
  $i = 0
  foreach ($page in $doc.Pages) {
    $i++
    $png = Join-Path $OutDir ("{0}-p{1}.png" -f $name, $i)
    if (Test-Path $png) { Remove-Item $png }
    $page.Export($png)
  }
  Write-Host "wrote $path ($i pages)"
}

$app = New-Object -ComObject Visio.InvisibleApp
try {
  $app.AlertResponse = 7  # answer "No" to any prompt instead of blocking

  # ---------------------------------------------------------------- basic.vsdx
  $doc = $app.Documents.Add("")
  $page = $doc.Pages.Item(1)
  $page.Name = "Shapes"
  Set-Formula $page.PageSheet "PageWidth" "11 in"
  Set-Formula $page.PageSheet "PageHeight" "8.5 in"

  $r = $page.DrawRectangle(0.5, 6, 3, 7.5)
  $r.Text = "Rectangle"
  Set-Formula $r "FillForegnd" "RGB(255,192,0)"
  Set-Formula $r "LineWeight" "2 pt"

  $o = $page.DrawOval(3.5, 6, 6, 7.5)
  $o.Text = "Oval"
  Set-Formula $o "FillForegnd" "RGB(91,155,213)"
  Set-Formula $o "Char.Color" "RGB(255,255,255)"

  $rot = $page.DrawRectangle(6.5, 6, 9, 7.5)
  $rot.Text = "Rotated 30"
  Set-Formula $rot "Angle" "30 deg"
  Set-Formula $rot "FillForegnd" "RGB(112,173,71)"

  $l = $page.DrawLine(0.5, 4.5, 3, 5.5)
  Set-Formula $l "EndArrow" "13"
  Set-Formula $l "BeginArrow" "4"
  Set-Formula $l "LineWeight" "1.5 pt"

  $d = $page.DrawLine(3.5, 4.5, 6, 5.5)
  Set-Formula $d "LinePattern" "2"
  Set-Formula $d "LineColor" "RGB(192,0,0)"

  $poly = $page.DrawPolyline([double[]](6.5, 4.5, 7.5, 5.5, 8.5, 4.5, 9.5, 5.5), 0)
  Set-Formula $poly "LineWeight" "2 pt"
  Set-Formula $poly "LineColor" "RGB(112,48,160)"

  $bez = $page.DrawBezier([double[]](0.5, 2.5, 1.5, 4, 2.5, 1, 3.5, 2.5), 3, 0)
  Set-Formula $bez "LineWeight" "2 pt"

  $arc = $page.DrawArcByThreePoints(4, 2, 6, 2, 5, 3)
  Set-Formula $arc "LineWeight" "2 pt"
  Set-Formula $arc "LineColor" "RGB(237,125,49)"

  $t = $page.DrawRectangle(6.5, 1.5, 10, 3.5)
  $t.Text = "Bold start, then plain text that wraps inside the box"
  Set-Formula $t "FillForegndTrans" "50%"
  Set-Formula $t "FillForegnd" "RGB(255,0,0)"
  Set-Formula $t "Para.HorzAlign" "0"
  Set-Formula $t "VerticalAlign" "0"
  $chars = $t.Characters
  $chars.Begin = 0; $chars.End = 10
  Set-CharProp $chars $visCharacterStyle 1
  Set-CharProp $chars $visCharacterColor 2

  $flip = $page.DrawPolyline([double[]](0.5, 0.3, 2.5, 0.3, 0.5, 1.3, 0.5, 0.3), 0)
  Set-Formula $flip "FillForegnd" "RGB(0,176,240)"
  Set-Formula $flip "FlipX" "TRUE"

  $g1 = $page.DrawRectangle(3.5, 0.3, 4.5, 1.2)
  $g2 = $page.DrawOval(4.7, 0.3, 5.7, 1.2)
  $sel = $page.CreateSelection($visSelTypeEmpty)
  $sel.Select($g1, $visSelect); $sel.Select($g2, $visSelect)
  $grp = $sel.Group()
  Set-Formula $grp "Angle" "-15 deg"
  $grp.Text = "Group"

  Export-Doc $doc "basic"
  $doc.Close()

  # -------------------------------------------------------------- masters.vsdx
  $doc = $app.Documents.Add("")
  $page = $doc.Pages.Item(1)
  $proto = $page.DrawRectangle(0, 0, 2, 1)
  Set-Formula $proto "FillForegnd" "RGB(68,114,196)"
  Set-Formula $proto "Char.Color" "RGB(255,255,255)"
  Set-Formula $proto "Rounding" "0.1 in"
  $proto.Text = "Step"
  $master = $doc.Masters.Drop($proto, 0, 0)
  $master.NameU = "Flow step"
  $proto.Delete()

  $shapes = @()
  for ($i = 0; $i -lt 4; $i++) {
    $s = $page.Drop($master, 1.5 + 2.5 * $i, 6)
    $s.Text = "Step $($i + 1)"
    $shapes += $s
  }
  Set-Formula $shapes[2] "FillForegnd" "RGB(237,125,49)"
  Set-Formula $shapes[3] "Width" "2.5 in"

  for ($i = 0; $i -lt 3; $i++) {
    $c = $page.Drop($app.ConnectorToolDataObject, 0, 0)
    $c.CellsU("BeginX").GlueTo($shapes[$i].CellsU("PinX"))
    $c.CellsU("EndX").GlueTo($shapes[$i + 1].CellsU("PinX"))
    Set-Formula $c "EndArrow" "4"
  }
  $c = $page.Drop($app.ConnectorToolDataObject, 0, 0)
  $c.CellsU("BeginX").GlueTo($shapes[0].CellsU("PinX"))
  $c.CellsU("EndX").GlueTo($shapes[3].CellsU("PinX"))
  Set-Formula $c "EndArrow" "4"
  Set-Formula $c "LineColor" "RGB(192,0,0)"

  # Background page + layers.
  $bg = $doc.Pages.Add()
  $bg.Name = "Background"
  $bg.Background = -1
  $band = $bg.DrawRectangle(0, 0, 8.5, 0.6)
  Set-Formula $band "FillForegnd" "RGB(217,217,217)"
  $band.Text = "Background page footer"
  $page.BackPage = $bg

  $hidden = $page.DrawRectangle(1, 2, 3, 3)
  $hidden.Text = "On hidden layer"
  $layer = $page.Layers.Add("Hidden notes")
  $layer.Add($hidden, 0)
  $layer.CellsC($visLayerVisible).FormulaU = "0"

  $note = $page.DrawRectangle(4, 2, 7, 3)
  $note.Text = "Visible note"
  Set-Formula $note "LinePattern" "0"
  Set-Formula $note "FillForegnd" "RGB(255,242,204)"

  Export-Doc $doc "masters"
  $doc.Close()

  # --------------------------------------------------------------- themed.vsdx
  # Theme colours, line and font styles come from THEMEVAL() (SPEC §11).
  $doc = $app.Documents.Add("")
  $page = $doc.Pages.Item(1)
  $boxes = @()
  for ($i = 0; $i -lt 3; $i++) {
    $b = $page.DrawRectangle(0.5 + 2.75 * $i, 6, 2.5 + 2.75 * $i, 7)
    $b.Text = "Themed $($i + 1)"
    $boxes += $b
  }
  Set-Formula $boxes[1] "QuickStyleFillColor" "4"
  Set-Formula $boxes[2] "QuickStyleFillMatrix" "3"
  Set-Formula $boxes[2] "QuickStyleLineMatrix" "3"
  $ov = $page.DrawOval(0.5, 3.5, 2.5, 5)
  $ov.Text = "Oval"
  for ($i = 0; $i -lt 2; $i++) {
    $c = $page.Drop($app.ConnectorToolDataObject, 0, 0)
    $c.CellsU("BeginX").GlueTo($boxes[$i].CellsU("PinX"))
    $c.CellsU("EndX").GlueTo($boxes[$i + 1].CellsU("PinX"))
  }
  $c = $page.Drop($app.ConnectorToolDataObject, 0, 0)
  $c.CellsU("BeginX").GlueTo($boxes[0].CellsU("PinX"))
  $c.CellsU("EndX").GlueTo($ov.CellsU("PinX"))
  $page.SetTheme("Linear")
  try { $page.SetThemeVariant(2, 2, 0) } catch { try { $page.SetThemeVariant(2, 2) } catch { Write-Host "no variants: $_" } }
  Export-Doc $doc "themed"
  $doc.Close()

  # --------------------------------------------------------------- images.vsdx
  # Foreign (image) shapes, using Flowster's own icon.
  $doc = $app.Documents.Add("")
  $page = $doc.Pages.Item(1)
  $icon = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "../../src-tauri/icons/128x128.png"))
  $img = $page.Import($icon)
  Set-Formula $img "PinX" "2 in"
  Set-Formula $img "PinY" "8 in"
  Set-Formula $img "Width" "2 in"
  Set-Formula $img "Height" "2 in"
  $img2 = $page.Import($icon)
  Set-Formula $img2 "PinX" "5 in"
  Set-Formula $img2 "PinY" "8 in"
  Set-Formula $img2 "Angle" "20 deg"
  $cap = $page.DrawRectangle(0.5, 6, 6.5, 6.6)
  $cap.Text = "Two imported images, one rotated"
  Set-Formula $cap "LinePattern" "0"
  Export-Doc $doc "images"
  $doc.Close()

  # ---------------------------------------------------------------- fills.vsdx
  # Gradient fills, fill patterns and shadows (SPEC §8.3).
  $doc = $app.Documents.Add("")
  $page = $doc.Pages.Item(1)
  Set-Formula $page.PageSheet "PageWidth" "11 in"
  Set-Formula $page.PageSheet "PageHeight" "8.5 in"
  $label = {
    param($shape, $text)
    $shape.Text = $text
  }

  # Modern gradients: FillGradientEnabled + stops.
  $g1 = $page.DrawRectangle(0.5, 6, 3, 7.5)
  & $label $g1 "Linear 0 deg"
  Set-Formula $g1 "FillGradientEnabled" "TRUE"
  Set-Formula $g1 "FillGradientDir" "0"
  Set-Formula $g1 "FillGradientAngle" "0 deg"
  Set-Formula $g1 "FillForegnd" "RGB(255,192,0)"
  $g2 = $page.DrawRectangle(3.5, 6, 6, 7.5)
  & $label $g2 "Linear 90 deg"
  Set-Formula $g2 "FillGradientEnabled" "TRUE"
  Set-Formula $g2 "FillGradientDir" "0"
  Set-Formula $g2 "FillGradientAngle" "90 deg"
  Set-Formula $g2 "FillForegnd" "RGB(91,155,213)"
  $g3 = $page.DrawOval(6.5, 6, 9, 7.5)
  & $label $g3 "Radial"
  Set-Formula $g3 "FillGradientEnabled" "TRUE"
  Set-Formula $g3 "FillGradientDir" "1"
  Set-Formula $g3 "FillForegnd" "RGB(112,173,71)"

  $stops = {
    param($shape, $list)
    for ($k = 0; $k -lt $list.Count; $k++) {
      $n = $k + 1
      Set-Formula $shape "FillGradientStops.GradientStopColor[$n]" $list[$k][0]
      Set-Formula $shape "FillGradientStops.GradientStopPosition[$n]" $list[$k][1]
      Set-Formula $shape "FillGradientStops.GradientStopColorTrans[$n]" "0%"
    }
  }
  & $stops $g1 @(@("RGB(255,192,0)", "0%"), @("RGB(192,0,0)", "100%"))
  & $stops $g2 @(@("RGB(91,155,213)", "0%"), @("RGB(255,255,255)", "60%"), @("RGB(31,78,121)", "100%"))
  & $stops $g3 @(@("RGB(255,255,255)", "0%"), @("RGB(112,173,71)", "100%"))

  # Legacy two-colour fill patterns and gradients (FillPattern 2-40).
  $patterns = @(2, 5, 10, 16, 25, 31, 36)
  for ($i = 0; $i -lt $patterns.Count; $i++) {
    $p = $page.DrawRectangle(0.5 + 1.4 * $i, 3.8, 1.7 + 1.4 * $i, 5.2)
    & $label $p "P$($patterns[$i])"
    Set-Formula $p "FillPattern" "$($patterns[$i])"
    Set-Formula $p "FillForegnd" "RGB(68,114,196)"
    Set-Formula $p "FillBkgnd" "RGB(255,242,204)"
  }

  # Shadows.
  $s1 = $page.DrawRectangle(0.5, 1, 3, 2.5)
  & $label $s1 "Shadow"
  Set-Formula $s1 "FillForegnd" "RGB(255,255,255)"
  Set-Formula $s1 "ShdwPattern" "1"
  Set-Formula $s1 "ShapeShdwType" "1"
  Set-Formula $s1 "ShapeShdwOffsetX" "0.1 in"
  Set-Formula $s1 "ShapeShdwOffsetY" "-0.1 in"
  Set-Formula $s1 "ShdwForegnd" "RGB(0,0,0)"
  Set-Formula $s1 "ShdwForegndTrans" "50%"
  $s2 = $page.DrawOval(3.5, 1, 6, 2.5)
  & $label $s2 "Blurred shadow"
  Set-Formula $s2 "FillForegnd" "RGB(237,125,49)"
  Set-Formula $s2 "ShdwPattern" "1"
  Set-Formula $s2 "ShapeShdwType" "1"
  Set-Formula $s2 "ShapeShdwOffsetX" "0.15 in"
  Set-Formula $s2 "ShapeShdwOffsetY" "-0.15 in"
  Set-Formula $s2 "ShapeShdwBlur" "8 pt"
  Set-Formula $s2 "ShdwForegnd" "RGB(80,80,80)"
  Set-Formula $s2 "ShdwForegndTrans" "30%"
  Export-Doc $doc "fills"
  $doc.Close()

  # ------------------------------------------------------------- patterns.vsdx
  # Every FillGradientDir (row 1) and every legacy FillPattern 2-40, labelled.
  $doc = $app.Documents.Add("")
  $page = $doc.Pages.Item(1)
  Set-Formula $page.PageSheet "PageWidth" "11 in"
  Set-Formula $page.PageSheet "PageHeight" "8.5 in"
  for ($d = 0; $d -le 13; $d++) {
    $x = 0.3 + 0.75 * $d
    $g = $page.DrawRectangle($x, 7.4, $x + 0.65, 8.1)
    $g.Text = "D$d"
    Set-Formula $g "FillGradientEnabled" "TRUE"
    Set-Formula $g "FillGradientDir" "$d"
    Set-Formula $g "FillGradientStops.GradientStopColor[1]" "RGB(255,255,255)"
    Set-Formula $g "FillGradientStops.GradientStopPosition[1]" "0%"
    Set-Formula $g "FillGradientStops.GradientStopColor[2]" "RGB(0,112,192)"
    Set-Formula $g "FillGradientStops.GradientStopPosition[2]" "100%"
  }
  for ($i = 2; $i -le 40; $i++) {
    $k = $i - 2
    $x = 0.3 + 1.05 * ($k % 10)
    $y = 6.0 - 1.4 * [math]::Floor($k / 10)
    $p = $page.DrawRectangle($x, $y, $x + 0.9, $y + 1.1)
    $p.Text = "$i"
    Set-Formula $p "FillPattern" "$i"
    Set-Formula $p "FillForegnd" "RGB(0,112,192)"
    Set-Formula $p "FillBkgnd" "RGB(255,255,255)"
  }
  Export-Doc $doc "patterns"
  $doc.Close()

  # ------------------------------------------------------------ gradients.vsdx
  # Borderless, text-free tiles for measuring gradient origins: row 1 is
  # FillGradientDir 1-13 (white -> blue stops), row 2 FillPattern 25-40 (white fg, blue bg).
  $doc = $app.Documents.Add("")
  $page = $doc.Pages.Item(1)
  Set-Formula $page.PageSheet "PageWidth" "22 in"
  Set-Formula $page.PageSheet "PageHeight" "4 in"
  for ($d = 1; $d -le 13; $d++) {
    $x = 0.2 + 1.6 * ($d - 1)
    $g = $page.DrawRectangle($x, 2.2, $x + 1.2, 3.4)
    Set-Formula $g "LinePattern" "0"
    Set-Formula $g "FillGradientEnabled" "TRUE"
    Set-Formula $g "FillGradientDir" "$d"
    Set-Formula $g "FillGradientStops.GradientStopColor[1]" "RGB(255,255,255)"
    Set-Formula $g "FillGradientStops.GradientStopPosition[1]" "0%"
    Set-Formula $g "FillGradientStops.GradientStopColor[2]" "RGB(0,0,255)"
    Set-Formula $g "FillGradientStops.GradientStopPosition[2]" "100%"
  }
  for ($i = 25; $i -le 40; $i++) {
    $x = 0.2 + 1.3 * ($i - 25)
    $p = $page.DrawRectangle($x, 0.4, $x + 1.0, 1.4)
    Set-Formula $p "LinePattern" "0"
    Set-Formula $p "FillPattern" "$i"
    Set-Formula $p "FillForegnd" "RGB(255,255,255)"
    Set-Formula $p "FillBkgnd" "RGB(0,0,255)"
  }
  Export-Doc $doc "gradients"
  $doc.Close()

  # ----------------------------------------------------------------- text.vsdx
  # Paragraph and vertical alignment, sizes, styles, fonts, spacing, indents,
  # rotated text and text backgrounds.
  $doc = $app.Documents.Add("")
  $page = $doc.Pages.Item(1)
  Set-Formula $page.PageSheet "PageWidth" "11 in"
  Set-Formula $page.PageSheet "PageHeight" "8.5 in"
  $long = "The quick brown fox jumps over the lazy dog, then naps in the sun."
  for ($a = 0; $a -le 3; $a++) {
    $x = 0.4 + 2.6 * $a
    $s = $page.DrawRectangle($x, 6.9, $x + 2.3, 8.1)
    $s.Text = $long
    Set-Formula $s "Para.HorzAlign" "$a"
  }
  for ($v = 0; $v -le 2; $v++) {
    $x = 0.4 + 2.0 * $v
    $s = $page.DrawRectangle($x, 5.0, $x + 1.7, 6.4)
    $s.Text = "VerticalAlign $v"
    Set-Formula $s "VerticalAlign" "$v"
  }
  $rt = $page.DrawRectangle(6.6, 5.2, 8.4, 6.2)
  $rt.Text = "Text at 30 degrees"
  Set-Formula $rt "TxtAngle" "30 deg"
  $bk = $page.DrawLine(8.8, 5.7, 10.6, 5.7)
  $bk.Text = "Label on a line"
  Set-Formula $bk "TextBkgnd" "RGB(255,230,153)"
  $sizes = 8, 12, 18, 24, 36
  for ($i = 0; $i -lt $sizes.Count; $i++) {
    $x = 0.4 + 2.1 * $i
    $s = $page.DrawRectangle($x, 3.4, $x + 1.9, 4.6)
    $s.Text = "$($sizes[$i]) pt"
    Set-Formula $s "Char.Size" "$($sizes[$i]) pt"
  }
  $styles = @(
    @("Italic", "Char.Style", "2"), @("Underline", "Char.Style", "4"), @("Bold italic", "Char.Style", "3"),
    @("Strikethrough", "Char.Strikethru", "TRUE"), @("Red text", "Char.Color", "RGB(192,0,0)")
  )
  for ($i = 0; $i -lt $styles.Count; $i++) {
    $x = 0.4 + 2.1 * $i
    $s = $page.DrawRectangle($x, 2.0, $x + 1.9, 3.0)
    $s.Text = $styles[$i][0]
    Set-Formula $s $styles[$i][1] $styles[$i][2]
    Set-Formula $s "Char.Size" "14 pt"
  }
  $fonts = "Times New Roman", "Consolas", "Georgia"
  for ($i = 0; $i -lt $fonts.Count; $i++) {
    $x = 0.4 + 2.1 * $i
    $s = $page.DrawRectangle($x, 0.3, $x + 1.9, 1.6)
    $s.Text = "$($fonts[$i]) font"
    Set-CharProp $s.Characters 0 $doc.Fonts.Item($fonts[$i]).ID
    Set-Formula $s "Char.Size" "14 pt"
  }
  $pp = $page.DrawRectangle(6.7, 0.3, 10.6, 1.8)
  $pp.Text = "First paragraph with a hanging indent that wraps.`nSecond paragraph after a gap."
  Set-Formula $pp "Para.HorzAlign" "0"
  Set-Formula $pp "Para.IndLeft" "0.25 in"
  Set-Formula $pp "Para.IndFirst" "-0.2 in"
  Set-Formula $pp "Para.SpBefore" "6 pt"
  Set-Formula $pp "Para.SpLine" "-150%"
  Export-Doc $doc "text"
  $doc.Close()

  # ---------------------------------------------------------------- lines.vsdx
  # LinePattern 1-23, weights, caps, arrowheads 1-24 at three sizes, rounding
  # and transparency. No text.
  $doc = $app.Documents.Add("")
  $page = $doc.Pages.Item(1)
  Set-Formula $page.PageSheet "PageWidth" "11 in"
  Set-Formula $page.PageSheet "PageHeight" "8.5 in"
  for ($i = 1; $i -le 23; $i++) {
    $y = 8.1 - 0.3 * ($i - 1)
    $l = $page.DrawLine(0.4, $y, 3.4, $y)
    Set-Formula $l "LinePattern" "$i"
    Set-Formula $l "LineWeight" "1.5 pt"
  }
  $weights = "0.25 pt", "0.5 pt", "1 pt", "2 pt", "4 pt", "8 pt"
  for ($i = 0; $i -lt $weights.Count; $i++) {
    $y = 8.1 - 0.35 * $i
    $l = $page.DrawLine(4.0, $y, 6.5, $y)
    Set-Formula $l "LineWeight" $weights[$i]
  }
  for ($c = 0; $c -le 2; $c++) {
    $y = 5.7 - 0.4 * $c
    $l = $page.DrawLine(4.2, $y, 6.3, $y)
    Set-Formula $l "LineWeight" "10 pt"
    Set-Formula $l "LineCap" "$c"
    Set-Formula $l "LineColor" "RGB(68,114,196)"
  }
  for ($i = 1; $i -le 24; $i++) {
    $col = [math]::Floor(($i - 1) / 12)
    $y = 8.1 - 0.32 * (($i - 1) % 12)
    $x = 7.0 + 2.0 * $col
    $l = $page.DrawLine($x, $y, $x + 1.6, $y)
    Set-Formula $l "BeginArrow" "$i"
    Set-Formula $l "EndArrow" "$i"
    Set-Formula $l "BeginArrowSize" "$($i % 3)"
    Set-Formula $l "EndArrowSize" "$(($i % 3) + 2)"
    Set-Formula $l "LineWeight" "1 pt"
  }
  $rr = $page.DrawRectangle(0.5, 0.5, 2.5, 1.8)
  Set-Formula $rr "Rounding" "0.25 in"
  Set-Formula $rr "LineWeight" "2 pt"
  $rp = $page.DrawPolyline([double[]](3.0, 0.5, 4.0, 1.8, 5.0, 0.5, 6.0, 1.8), 0)
  Set-Formula $rp "Rounding" "0.2 in"
  Set-Formula $rp "LineWeight" "2 pt"
  $tr = $page.DrawRectangle(6.5, 0.5, 8.5, 1.8)
  Set-Formula $tr "LineWeight" "6 pt"
  Set-Formula $tr "LineColor" "RGB(192,0,0)"
  Set-Formula $tr "LineColorTrans" "50%"
  Set-Formula $tr "LinePattern" "2"
  Export-Doc $doc "lines"
  $doc.Close()

  # --------------------------------------------------------------- arrows.vsdx
  # EndArrow 1-45 (rows) at EndArrowSize 0-6 on 1 pt lines, then size 2 on
  # 3 pt and 6 pt lines (columns), and a bar fixing the scale.
  $doc = $app.Documents.Add("")
  $page = $doc.Pages.Item(1)
  Set-Formula $page.PageSheet "PageWidth" "18 in"
  Set-Formula $page.PageSheet "PageHeight" "28 in"
  $cols = @(@(0, "1 pt"), @(1, "1 pt"), @(2, "1 pt"), @(3, "1 pt"), @(4, "1 pt"), @(5, "1 pt"), @(6, "1 pt"), @(2, "3 pt"), @(2, "6 pt"))
  for ($t = 1; $t -le 45; $t++) {
    $y = 27.5 - 0.6 * ($t - 1)
    for ($c = 0; $c -lt $cols.Count; $c++) {
      $x = 0.5 + 1.9 * $c
      $l = $page.DrawLine($x, $y, $x + 1.5, $y)
      Set-Formula $l "EndArrow" "$t"
      Set-Formula $l "EndArrowSize" "$($cols[$c][0])"
      Set-Formula $l "LineWeight" $cols[$c][1]
    }
  }
  $bar = $page.DrawRectangle(0.5, 0.2, 17.5, 0.3)
  Set-Formula $bar "LinePattern" "0"
  Set-Formula $bar "FillForegnd" "RGB(0,0,0)"
  Export-Doc $doc "arrows"
  $doc.Close()

  # ---------------------------------------------------------------- fonts.vsdx
  # Unwrapped lines in three fonts at 12 and 36 pt: a sentence with kerning
  # pairs and runs of narrow and wide letters, for measuring advance widths.
  $doc = $app.Documents.Add("")
  $page = $doc.Pages.Item(1)
  Set-Formula $page.PageSheet "PageWidth" "16 in"
  Set-Formula $page.PageSheet "PageHeight" "8 in"
  $texts = "The quick brown fox jumps over the lazy dog. AV To Wa", ("i" * 40), ("m" * 30)
  $row = 0
  foreach ($f in "Calibri", "Times New Roman", "Arial") {
    foreach ($sz in "12 pt", "36 pt") {
      foreach ($t in $texts) {
        $y = 7.6 - 0.37 * $row
        $s = $page.DrawRectangle(0.5, $y - 0.15, 15.5, $y + 0.15)
        $s.Text = $t
        Set-Formula $s "LinePattern" "0"
        Set-Formula $s "FillPattern" "0"
        Set-Formula $s "Para.HorzAlign" "0"
        Set-Formula $s "Char.Size" $sz
        Set-CharProp $s.Characters 0 $doc.Fonts.Item($f).ID
        $row++
      }
    }
  }
  $bar = $page.DrawRectangle(0.5, 0.2, 15.5, 0.3)
  Set-Formula $bar "LinePattern" "0"
  Set-Formula $bar "FillForegnd" "RGB(0,0,0)"
  Export-Doc $doc "fonts"
  $doc.Close()

  # ------------------------------------------------------------------ emf.vsdx
  # Our own EMF (GDI records only), drawn with System.Drawing, imported twice:
  # at its natural size and stretched.
  $emf = Join-Path $OutDir "flowster-test.emf"
  New-TestEmf $emf
  $doc = $app.Documents.Add("")
  $page = $doc.Pages.Item(1)
  Set-Formula $page.PageSheet "PageWidth" "11 in"
  Set-Formula $page.PageSheet "PageHeight" "8.5 in"
  $a = $page.Import($emf)
  Set-Formula $a "PinX" "3 in"
  Set-Formula $a "PinY" "5.5 in"
  $b = $page.Import($emf)
  Set-Formula $b "PinX" "8 in"
  Set-Formula $b "PinY" "3 in"
  Set-Formula $b "Width" "4 in"
  Set-Formula $b "Height" "2 in"
  $gdi = Join-Path $OutDir "flowster-gdi.emf"
  New-TestEmfGdi $gdi
  $c = $page.Import($gdi)
  Set-Formula $c "PinX" "3 in"
  Set-Formula $c "PinY" "1.8 in"
  Export-Doc $doc "emf"
  $doc.Close()
}
finally {
  $app.Quit()
  [void][Runtime.InteropServices.Marshal]::ReleaseComObject($app)
}
