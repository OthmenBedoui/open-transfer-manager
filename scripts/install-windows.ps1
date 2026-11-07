<# Build a per-user Windows setup and launch its wizard. No Python or WebView. #>
[CmdletBinding()]
param(
    [switch]$BuildOnly,
    [string]$IsccPath,
    [string]$MediaToolsDirectory,
    [string]$ChromeWebStoreId
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$projectRoot = Split-Path -Parent $PSScriptRoot
if ($ChromeWebStoreId -and $ChromeWebStoreId -cnotmatch '^[a-p]{32}$') { throw 'ChromeWebStoreId must be the 32-letter ID of your published extension.' }
if ($env:OS -ne 'Windows_NT') { throw 'Windows setup must be built on Windows.' }
# Respect the installed default Rust toolchain (MSVC recommended; GNU supported).
$cargo = Get-Command cargo -ErrorAction SilentlyContinue
if (-not $cargo -and (Test-Path "$env:USERPROFILE/.cargo/bin/cargo.exe")) {
    $env:PATH = "$env:USERPROFILE/.cargo/bin;" + $env:PATH
    $cargo = Get-Command cargo
}
if (-not $cargo) { throw 'Install Rust with rustup, then run this command again.' }
if (-not (Get-Command gcc -ErrorAction SilentlyContinue)) {
    $wingetPackages = Join-Path $env:LOCALAPPDATA 'Microsoft/WinGet/Packages'
    if (Test-Path $wingetPackages) {
        $gnuBin = Get-ChildItem -LiteralPath $wingetPackages -Directory -Filter 'BrechtSanders.WinLibs*' |
            ForEach-Object { Join-Path $_.FullName 'mingw64/bin' } |
            Where-Object { Test-Path (Join-Path $_ 'gcc.exe') } | Select-Object -First 1
        if ($gnuBin) { $env:PATH = "$gnuBin;" + $env:PATH }
    }
}
if (-not $IsccPath) {
    $candidates = @("${env:ProgramFiles(x86)}/Inno Setup 6/ISCC.exe", "$env:ProgramFiles/Inno Setup 7/ISCC.exe", "$env:LOCALAPPDATA/Programs/Inno Setup 6/ISCC.exe", "$env:LOCALAPPDATA/Programs/Inno Setup 7/ISCC.exe")
    $compiler = Get-Command ISCC.exe -ErrorAction SilentlyContinue
    if ($compiler) { $IsccPath = $compiler.Source }
    else { $IsccPath = $candidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1 }
    if (-not $IsccPath) {
        foreach ($hive in @('HKCU:', 'HKLM:', 'HKLM:\SOFTWARE\WOW6432Node')) {
            foreach ($major in @(6, 7)) {
                $key = if ($hive -like '*WOW6432Node') { "$hive\Microsoft\Windows\CurrentVersion\Uninstall\Inno Setup ${major}_is1" } else { "$hive\Software\Microsoft\Windows\CurrentVersion\Uninstall\Inno Setup ${major}_is1" }
                $registration = Get-ItemProperty -LiteralPath $key -ErrorAction SilentlyContinue
                if ($registration -and $registration.PSObject.Properties['InstallLocation']) {
                    $candidate = Join-Path $registration.InstallLocation 'ISCC.exe'
                    if (Test-Path -LiteralPath $candidate) { $IsccPath = $candidate; break }
                }
            }
            if ($IsccPath) { break }
        }
    }
}
if (-not $IsccPath -or -not (Test-Path -LiteralPath $IsccPath)) {
    throw 'Install Inno Setup 6/7 (winget install JRSoftware.InnoSetup) or pass -IsccPath C:\path\ISCC.exe.'
}
Push-Location $projectRoot
$savedRustFlags = $env:RUSTFLAGS
try {
    & $cargo.Source test --locked
    if ($LASTEXITCODE -ne 0) { throw 'Rust tests failed' }
    $hostInfo = & rustc -vV
    if ($hostInfo -match 'host: .*windows-msvc') {
        $env:RUSTFLAGS = ($savedRustFlags + ' -C target-feature=+crt-static').Trim()
    }
    & $cargo.Source build --release --locked
    if ($LASTEXITCODE -ne 0) { throw 'Rust release build failed' }
    & "$PSScriptRoot/build-extension.ps1" -ChromeWebStoreId $ChromeWebStoreId
    $bundle = [IO.Path]::GetFullPath((Join-Path $projectRoot 'dist/windows'))
    New-Item -ItemType Directory -Force -Path "$bundle/payload" | Out-Null
    # Bundle only GNU runtimes actually imported, rather than shipping unused C++ DLLs.
    $gcc = Get-Command gcc -ErrorAction SilentlyContinue
    $runtimeNames = @('libgcc_s_seh-1.dll', 'libstdc++-6.dll', 'libwinpthread-1.dll')
    $imports = ''
    $objdump = $null
    if ($gcc) {
        $objdump = Join-Path (Split-Path $gcc.Source) 'objdump.exe'
        if (Test-Path -LiteralPath $objdump) { $imports = (& $objdump -p target/release/otm.exe | Out-String) }
    }
    foreach ($name in $runtimeNames) {
        $runtime = Join-Path "$bundle/payload" $name
        if (Test-Path -LiteralPath $runtime) { Remove-Item -LiteralPath $runtime -Force }
        if ($gcc -and $imports -match [regex]::Escape($name)) {
            $source = Join-Path (Split-Path $gcc.Source) $name
            if (Test-Path -LiteralPath $source) { Copy-Item -LiteralPath $source -Destination $runtime }
        }
    }
    # Optional offline media tools; the default installer remains small.
    foreach ($name in @('yt-dlp.exe', 'ffmpeg.exe', 'ffprobe.exe')) {
        $destination = Join-Path "$bundle/payload" $name
        if (Test-Path -LiteralPath $destination) { Remove-Item -LiteralPath $destination -Force }
        if ($MediaToolsDirectory -and (Test-Path -LiteralPath (Join-Path $MediaToolsDirectory $name))) {
            Copy-Item -LiteralPath (Join-Path $MediaToolsDirectory $name) -Destination $destination
        }
    }
    $version = [regex]::Match((Get-Content Cargo.toml -Raw), '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
    # ICO container around the existing PNG. No image converter or Python dependency.
    $png = [IO.File]::ReadAllBytes((Join-Path $projectRoot 'assets/otm-64.png'))
    $stream = [IO.File]::Create((Join-Path $bundle 'otm.ico'))
    $writer = New-Object IO.BinaryWriter($stream)
    try {
        $writer.Write([uint16]0); $writer.Write([uint16]1); $writer.Write([uint16]1)
        $writer.Write([byte]64); $writer.Write([byte]64); $writer.Write([byte]0); $writer.Write([byte]0)
        $writer.Write([uint16]1); $writer.Write([uint16]32); $writer.Write([uint32]$png.Length); $writer.Write([uint32]22)
        $writer.Write($png)
    } finally { $writer.Dispose() }
    $definitions = @("/DAppVersion=$version")
    if ($ChromeWebStoreId) { $definitions += "/DChromeStoreId=$ChromeWebStoreId" }
    & $IsccPath @definitions "$projectRoot/installer/windows.iss"
    if ($LASTEXITCODE -ne 0) { throw 'Installer compilation failed' }
    $setup = Join-Path $bundle "OTM-Setup-$version.exe"
    Get-FileHash -LiteralPath $setup -Algorithm SHA256 | Format-List
    if (-not $BuildOnly) { Start-Process -FilePath $setup -Wait }
    Write-Host "Setup ready: $setup"
} finally { $env:RUSTFLAGS = $savedRustFlags; Pop-Location }
