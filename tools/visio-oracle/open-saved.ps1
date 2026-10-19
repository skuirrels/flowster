<#
.SYNOPSIS
  Visio oracle for Save (SPEC §19.2, §22.3): opens every .vsdx in a folder with
  Visio and exports each page as <name>-visio-p<N>.png, so Visio's rendering of
  a file Flowster wrote can be compared with its rendering of the original.

  pwsh tools/visio-oracle/open-saved.ps1 [-Dir target/corpus-saved]

  A file Visio refuses to open is reported and the run continues.
#>
param(
  [string]$Dir = (Join-Path $PSScriptRoot "..\..\target\corpus-saved")
)
$ErrorActionPreference = "Stop"
$Dir = [System.IO.Path]::GetFullPath($Dir)
$app = New-Object -ComObject Visio.InvisibleApp
try {
  $app.AlertResponse = 7
  foreach ($file in Get-ChildItem -Path $Dir -Filter *.vsdx) {
    try {
      $doc = $app.Documents.OpenEx($file.FullName, 2)  # visOpenRO
    } catch {
      Write-Host ("FAIL open {0}: {1}" -f $file.Name, $_.Exception.Message)
      continue
    }
    $i = 0
    foreach ($page in $doc.Pages) {
      $i++
      $png = Join-Path $Dir ("{0}-visio-p{1}.png" -f $file.BaseName, $i)
      if (Test-Path $png) { Remove-Item $png }
      $page.Export($png)
    }
    Write-Host ("opened {0}: {1} pages, {2} shapes on page 1" -f $file.Name, $doc.Pages.Count, $doc.Pages.Item(1).Shapes.Count)
    $doc.Saved = $true
    $doc.Close()
  }
}
finally {
  $app.Quit()
}
