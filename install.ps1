# Open Transfer Manager - one-command install for Windows 10 / 11.
#
#   irm https://raw.githubusercontent.com/OthmenBedoui/open-transfer-manager/main/install.ps1 | iex
#
# Downloads the Windows setup of the latest release, checks it against the release's
# SHA256SUMS.txt, installs it for the current user (no admin rights), and lets the setup
# download yt-dlp, ffmpeg and rclone (each checked against its official checksum).
# Testing hook: $env:OTM_SETUP = 'C:\path\OTM-Setup-x.y.z.exe' installs that file instead.

& {
    $ErrorActionPreference = 'Stop'
    $repo = 'OthmenBedoui/open-transfer-manager'
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    function Step($t) { Write-Host "==> $t" -ForegroundColor Cyan }
    function Ok($t) { Write-Host "  ok $t" -ForegroundColor Green }

    $dir = Join-Path $env:TEMP ('otm-install-' + [guid]::NewGuid())
    New-Item -ItemType Directory -Path $dir | Out-Null
    try {
        if ($env:OTM_SETUP) {
            Step "Installing local file $env:OTM_SETUP"
            $exe = $env:OTM_SETUP
        } else {
            Step 'Finding the latest release'
            $headers = @{ 'User-Agent' = 'otm-installer' }
            $rel = Invoke-RestMethod "https://api.github.com/repos/$repo/releases/latest" -Headers $headers
            $setup = $rel.assets | Where-Object { $_.name -like 'OTM-Setup-*.exe' } | Select-Object -First 1
            $sums = $rel.assets | Where-Object { $_.name -eq 'SHA256SUMS.txt' } | Select-Object -First 1
            if (-not $setup -or -not $sums) { throw 'The latest release has no Windows setup.' }
            Ok $rel.name

            Step "Downloading $($setup.name)"
            $exe = Join-Path $dir $setup.name
            Invoke-WebRequest $setup.browser_download_url -OutFile $exe -UseBasicParsing
            $sumFile = Join-Path $dir 'SHA256SUMS.txt'
            Invoke-WebRequest $sums.browser_download_url -OutFile $sumFile -UseBasicParsing
            $line = Get-Content $sumFile | Where-Object { $_ -match ('\s\*?' + [regex]::Escape($setup.name) + '\s*$') } | Select-Object -First 1
            if (-not $line) { throw "No checksum published for $($setup.name)." }
            $want = ($line.Trim() -split '\s+')[0].ToLower()
            $got = (Get-FileHash $exe -Algorithm SHA256).Hash.ToLower()
            if ($got -ne $want) { throw 'Checksum mismatch: the download is damaged or was changed. Nothing was installed.' }
            Ok 'checksum verified'
        }

        $running = Get-Process otm -ErrorAction SilentlyContinue
        if ($running) {
            Step 'Closing the running Open Transfer Manager (your downloads are kept)'
            $running | Stop-Process -Force
            Start-Sleep -Seconds 2
        }
        Step 'Installing (and downloading yt-dlp, ffmpeg and rclone: a few minutes)'
        $p = Start-Process $exe -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/TASKS=tools' -Wait -PassThru
        if ($p.ExitCode -ne 0) { throw "Setup failed (exit code $($p.ExitCode))." }
    } finally {
        Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
    }

    $otm = Join-Path $env:LOCALAPPDATA 'Programs\Open Transfer Manager\otm.exe'
    if (-not (Test-Path $otm)) { throw "Setup finished but $otm is missing." }
    Write-Host ''
    Write-Host 'Open Transfer Manager is installed.' -ForegroundColor Green
    Write-Host '  Open it from the Start menu (Open Transfer Manager).'
    Write-Host "  Check everything:  & '$otm' doctor"
    Write-Host ''
}
