<# Tests a separately identified installer without uninstalling the user's real OTM. #>
[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][string]$IsccPath,
    [string]$ScreenshotPath
)
$ErrorActionPreference='Stop'
$root=Split-Path -Parent $PSScriptRoot
$caseRoot=Join-Path ([IO.Path]::GetTempPath()) ('OTM-Setup-Test-'+[guid]::NewGuid().ToString('N'))
$app=Join-Path $caseRoot 'app'
$data=Join-Path $caseRoot 'state'
$testIdentity='OTM.InstallationSmoke'
$uninstallKey="HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\${testIdentity}_is1"
if(Test-Path -LiteralPath $uninstallKey){throw 'Another OTM installation smoke test is already installed.'}
New-Item -ItemType Directory -Force -Path $caseRoot | Out-Null
& $IsccPath /Qp "/DAppIdentity=$testIdentity" "/O$caseRoot" /FOTM-Setup-Test "$root/installer/windows.iss"
if($LASTEXITCODE -ne 0){throw 'Test installer compilation failed'}
$keys=@('Software\Google\Chrome\NativeMessagingHosts','Software\Microsoft\Edge\NativeMessagingHosts','Software\BraveSoftware\Brave-Browser\NativeMessagingHosts','Software\Mozilla\NativeMessagingHosts')
$saved=@()
foreach($key in $keys){
    $path="Registry::HKEY_CURRENT_USER\$key\org.opentransfermanager.host"
    $exists=Test-Path -LiteralPath $path
    $values=@()
    if($exists){
        $reg=Get-Item -LiteralPath $path
        foreach($name in $reg.GetValueNames()){
            $values+=@{Name=$name;Value=$reg.GetValue($name,$null,[Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames);Kind=$reg.GetValueKind($name).ToString()}
        }
    }
    $saved+=@{Path=$path;Exists=$exists;Values=$values}
}
$oldData=$env:OTM_DATA_DIR
$env:OTM_DATA_DIR=$data
function Remove-TestInstallation {
    $uninstaller=Join-Path $app 'unins000.exe'
    if(-not(Test-Path -LiteralPath $uninstaller)){return}
    $launcher=Join-Path $app 'Desinstaller OTM.exe'
    $command=if(Test-Path -LiteralPath $launcher){$launcher}else{$uninstaller}
    $p=Start-Process -FilePath $command -ArgumentList @('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART',('/LOG="'+(Join-Path $caseRoot 'uninstall.log')+'"')) -PassThru -Wait -WindowStyle Hidden
    if($p.ExitCode -ne 0){throw "Uninstall launcher failed: $($p.ExitCode)"}
    $deadline=[DateTime]::UtcNow.AddSeconds(30)
    while((Test-Path -LiteralPath $uninstaller) -and [DateTime]::UtcNow -lt $deadline){Start-Sleep -Milliseconds 200}
    foreach($file in @('otm.exe','Desinstaller OTM.exe','unins000.exe')){
        if(Test-Path -LiteralPath (Join-Path $app $file)){throw "Uninstall left executable behind: $file"}
    }
}
try {
    $p=Start-Process -FilePath (Join-Path $caseRoot 'OTM-Setup-Test.exe') -ArgumentList @('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART','/NOICONS',('/GROUP="OTM Installation Smoke"'),('/DIR="'+$app+'"'),('/LOG="'+(Join-Path $caseRoot 'install.log')+'"')) -PassThru -Wait -WindowStyle Hidden
    if($p.ExitCode -ne 0){throw "Setup failed: $($p.ExitCode)"}
    $exe=Join-Path $app 'otm.exe'
    foreach($file in @('otm.exe','Desinstaller OTM.exe','unins000.exe','extensions/distribution.json','extensions/chromium/manifest.json','extensions/firefox/manifest.json','licenses/NotoSansArabic-OFL.txt')){
        if(-not(Test-Path -LiteralPath (Join-Path $app $file))){throw "Missing installed file: $file"}
    }
    if((Get-FileHash $exe).Hash -ne (Get-FileHash "$root/target/release/otm.exe").Hash){throw 'Installed main binary differs from the release build'}
    $manifest=Get-Content -LiteralPath "$app/extensions/chromium/manifest.json" -Raw | ConvertFrom-Json
    $sha=[Security.Cryptography.SHA256]::Create()
    try{$hash=$sha.ComputeHash([Convert]::FromBase64String($manifest.key))}finally{$sha.Dispose()}
    $id=-join($hash[0..15]|ForEach-Object{[char](97+($_ -shr 4));[char](97+($_ -band 15))})
    $hostManifest=Get-Content -LiteralPath "$data/org.opentransfermanager.host.chromium.json" -Raw | ConvertFrom-Json
    if($hostManifest.path -ne $exe -or $hostManifest.allowed_origins -notcontains "chrome-extension://$id/"){throw 'Chrome extension ID or installed native-host path mismatch'}
    foreach($arg in @('native-host',"chrome-extension://$id/")){
        $info=New-Object Diagnostics.ProcessStartInfo
        $info.FileName=$exe;$info.Arguments=$arg;$info.UseShellExecute=$false;$info.CreateNoWindow=$true
        $info.RedirectStandardInput=$true;$info.RedirectStandardOutput=$true;$info.RedirectStandardError=$true
        $native=New-Object Diagnostics.Process
        $native.StartInfo=$info;$native.Start()|Out-Null
        $bytes=[Text.Encoding]::UTF8.GetBytes('{"id":7,"cmd":"ping"}')
        $header=[BitConverter]::GetBytes([uint32]$bytes.Length)
        $native.StandardInput.BaseStream.Write($header,0,4)
        $native.StandardInput.BaseStream.Write($bytes,0,$bytes.Length)
        $native.StandardInput.Close()
        if(-not $native.WaitForExit(10000)){$native.Kill();throw 'Native host timed out'}
        $buffer=New-Object IO.MemoryStream
        $native.StandardOutput.BaseStream.CopyTo($buffer)
        $reply=$buffer.ToArray()
        if($reply.Length -le 4){throw 'Native host returned no framed response'}
        $length=[BitConverter]::ToUInt32($reply,0)
        $message=[Text.Encoding]::UTF8.GetString($reply,4,$length)|ConvertFrom-Json
        if(-not $message.ok -or $message.id -ne 7){throw 'Native host roundtrip failed'}
        Write-Output "Native Messaging OK: $arg"
    }
    if($ScreenshotPath){
        $p=Start-Process -FilePath $exe -ArgumentList @('--setup-browsers','--screenshot',('"'+$ScreenshotPath+'"')) -PassThru -WindowStyle Hidden
        if(-not $p.WaitForExit(20000)){$p.Kill();throw 'Native UI screenshot timed out'}
        if(-not(Test-Path -LiteralPath $ScreenshotPath)){throw 'Native UI screenshot missing'}
    }
    Remove-TestInstallation
    if(Test-Path -LiteralPath $uninstallKey){throw 'Windows uninstall metadata was not removed'}
    foreach($record in $saved){if(Test-Path -LiteralPath $record.Path){throw 'Test browser bridge was not removed'}}
    Write-Output 'Installer, matching Chrome/native-host IDs, native messaging and Desinstaller OTM.exe passed.'
} finally {
    try {Remove-TestInstallation} finally {
        foreach($record in $saved){
            if($record.Exists){
                New-Item -Path $record.Path -Force|Out-Null
                foreach($value in $record.Values){
                    if($value.Name -eq ''){Set-Item -LiteralPath $record.Path -Value $value.Value}
                    else{New-ItemProperty -LiteralPath $record.Path -Name $value.Name -Value $value.Value -PropertyType $value.Kind -Force|Out-Null}
                }
            }elseif(Test-Path -LiteralPath $record.Path){Remove-Item -LiteralPath $record.Path -Force}
        }
        $env:OTM_DATA_DIR=$oldData
        Write-Output "Test logs: $caseRoot"
    }
}
