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

# Visio constants (documented in the Visio type library; see docs/re/enums.txt).
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
}
finally {
  $app.Quit()
  [void][Runtime.InteropServices.Marshal]::ReleaseComObject($app)
}
