<#
.SYNOPSIS
  Visio oracle (SPEC §22.3): records how Visio evaluates Flowster's formula
  test cases, as the conformance fixture for the formula engine.

.DESCRIPTION
  Builds a context shape (Sheet.1) and a second shape (Sheet.2), then for each
  line of formulas.txt sets User.Test.Value to the formula and records Visio's
  ResultIU (internal units), ResultStr and error state. Writes
  crates/flowster-core/tests/fixtures/formulas.json, which is committed: it holds
  our formulas and Visio's computed results only.

  pwsh tools/visio-oracle/formula-fixtures.ps1
#>
$ErrorActionPreference = "Stop"
$root = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "../.."))
$cases = Get-Content (Join-Path $PSScriptRoot "formulas.txt") | Where-Object { $_.Trim() -and -not $_.TrimStart().StartsWith("#") }
$out = Join-Path $root "crates/flowster-core/tests/fixtures/formulas.json"
New-Item -ItemType Directory -Force (Split-Path $out) | Out-Null

$visSectionProp = 243
$visSectionUser = 242
$visSectionScratch = 6
$visRowLast = -2
$visTagDefault = 0

$app = New-Object -ComObject Visio.InvisibleApp
try {
  $app.AlertResponse = 7
  $doc = $app.Documents.Add("")
  $page = $doc.Pages.Item(1)
  $s = $page.DrawRectangle(1, 1, 3, 2)      # Sheet.1: 2 × 1 in, pin (2, 1.5)
  $s2 = $page.DrawRectangle(5, 5, 8, 6.5)   # Sheet.2: 3 × 1.5 in
  $s.CellsU("Angle").FormulaU = "30 deg"
  $s.CellsU("Char.Size").FormulaU = "14 pt"

  [void]$s.AddNamedRow($visSectionProp, "Cost", $visTagDefault)
  $s.CellsU("Prop.Cost.Value").FormulaU = "12.5"
  [void]$s.AddNamedRow($visSectionProp, "Name", $visTagDefault)
  $s.CellsU("Prop.Name.Value").FormulaU = '"Widget"'
  [void]$s.AddNamedRow($visSectionUser, "Base", $visTagDefault)
  $s.CellsU("User.Base.Value").FormulaU = "4"
  [void]$s.AddNamedRow($visSectionUser, "Test", $visTagDefault)
  [void]$s.AddSection($visSectionScratch)
  [void]$s.AddRow($visSectionScratch, $visRowLast, $visTagDefault)
  $s.CellsU("Scratch.X1").FormulaU = "1.25"
  $s.CellsU("Scratch.Y1").FormulaU = "0.5"

  $context = [ordered]@{}
  foreach ($c in @("Width", "Height", "PinX", "PinY", "LocPinX", "LocPinY", "Angle", "Prop.Cost", "User.Base",
                   "Scratch.X1", "Scratch.Y1", "Geometry1.X3", "Geometry1.Y3", "Char.Size")) {
    $context[$c] = $s.CellsU($c).ResultIU
  }
  $context["Prop.Name"] = $s.CellsU("Prop.Name").ResultStr("")
  $context["Sheet.2!Width"] = $s2.CellsU("Width").ResultIU

  $cell = $s.CellsU("User.Test.Value")
  $results = @()
  foreach ($f in $cases) {
    $r = [ordered]@{ formula = $f }
    try {
      $cell.FormulaForceU = $f
      $r.value = $cell.ResultIU
      $r.str = $cell.ResultStr("")
      $r.error = $cell.Error
    } catch {
      $r.parseError = $_.Exception.Message
    }
    $results += [pscustomobject]$r
  }
  $json = [ordered]@{
    generator = "tools/visio-oracle/formula-fixtures.ps1"
    visio = $app.Version + " build " + $app.Build
    context = $context
    cases = $results
  } | ConvertTo-Json -Depth 5
  [System.IO.File]::WriteAllText($out, $json + "`n")
  Write-Host "wrote $out ($($results.Count) cases)"
  $doc.Close()
}
finally {
  $app.Quit()
}
