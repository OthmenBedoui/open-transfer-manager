[CmdletBinding()]
param([string]$ChromeWebStoreId)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$version = [regex]::Match((Get-Content -LiteralPath "$projectRoot/Cargo.toml" -Raw), '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
if (-not $version) { throw 'Cargo package version not found' }
if ($ChromeWebStoreId -and $ChromeWebStoreId -cnotmatch '^[a-p]{32}$') { throw 'ChromeWebStoreId must be the 32-letter ID of your published Chrome Web Store extension.' }
$outputRoot = [IO.Path]::GetFullPath((Join-Path $projectRoot 'dist/extension'))
New-Item -ItemType Directory -Force -Path $outputRoot | Out-Null
$distribution = @{ chrome_store_id = $(if ($ChromeWebStoreId) { $ChromeWebStoreId } else { $null }) } | ConvertTo-Json
[IO.File]::WriteAllText((Join-Path $outputRoot 'distribution.json'), $distribution, (New-Object Text.UTF8Encoding($false)))
foreach ($family in @('chromium', 'firefox')) {
    $manifest = Get-Content -LiteralPath "$projectRoot/extension/manifest.base.json" -Raw | ConvertFrom-Json
    $extra = Get-Content -LiteralPath "$projectRoot/extension/manifest.$family.json" -Raw | ConvertFrom-Json
    foreach ($property in $extra.PSObject.Properties) {
        $manifest | Add-Member -NotePropertyName $property.Name -NotePropertyValue $property.Value -Force
    }
    $manifest.version = $version
    $destination = [IO.Path]::GetFullPath((Join-Path $outputRoot $family))
    if (-not $destination.StartsWith($outputRoot + [IO.Path]::DirectorySeparatorChar)) { throw 'Invalid extension output' }
    if (Test-Path -LiteralPath $destination) { Remove-Item -LiteralPath $destination -Recurse -Force }
    Copy-Item -LiteralPath "$projectRoot/extension/src" -Destination $destination -Recurse
    $json = $manifest | ConvertTo-Json -Depth 30
    [IO.File]::WriteAllText((Join-Path $destination 'manifest.json'), $json, (New-Object Text.UTF8Encoding($false)))
    $zip = Join-Path $outputRoot "otm-$family-$version.zip"
    Compress-Archive -Path "$destination/*" -DestinationPath $zip -Force
    Write-Host "Extension $family : $zip"
}
