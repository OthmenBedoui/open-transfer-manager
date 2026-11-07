#ifndef AppVersion
  #define AppVersion "0.2.5"
#endif
#ifndef AppIdentity
  #define AppIdentity "{{F390B4CB-96AC-4D1E-9302-4CC6F06443BB}"
#endif
#define Root AddBackslash(SourcePath) + ".."
[Setup]
AppId={#AppIdentity}
AppName=Open Transfer Manager
AppVersion={#AppVersion}
AppPublisher=Open Transfer Manager contributors
AppPublisherURL=https://github.com/OthmenBedoui/open-transfer-manager
#ifdef ChromeStoreId
DefaultDirName={autopf}\Open Transfer Manager
#else
DefaultDirName={localappdata}\Programs\Open Transfer Manager
#endif
DefaultGroupName=Open Transfer Manager
#ifdef ChromeStoreId
PrivilegesRequired=admin
#else
PrivilegesRequired=lowest
#endif
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.17763
OutputDir={#Root}\dist\windows
OutputBaseFilename=OTM-Setup-{#AppVersion}
SetupIconFile={#Root}\dist\windows\otm.ico
UninstallDisplayIcon={app}\otm.ico
LicenseFile={#Root}\LICENSE
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
RestartApplications=no
AppMutex=OpenTransferManagerSetup
[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "french"; MessagesFile: "compiler:Languages\French.isl"
[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked
[Files]
Source: "{#Root}\target\release\otm.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Root}\target\release\otm-uninstall.exe"; DestDir: "{app}"; DestName: "Desinstaller OTM.exe"; Flags: ignoreversion
Source: "{#Root}\dist\windows\otm.ico"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Root}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Root}\assets\fonts\Inter-OFL.txt"; DestDir: "{app}\licenses"; Flags: ignoreversion
Source: "{#Root}\assets\fonts\NotoSansArabic-OFL.txt"; DestDir: "{app}\licenses"; Flags: ignoreversion
Source: "{#Root}\docs\BROWSER_EXTENSIONS.md"; DestDir: "{app}\extensions"; Flags: ignoreversion
Source: "{#Root}\dist\extension\chromium\*"; DestDir: "{app}\extensions\chromium"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#Root}\dist\extension\firefox\*"; DestDir: "{app}\extensions\firefox"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#Root}\dist\extension\distribution.json"; DestDir: "{app}\extensions"; Flags: ignoreversion
Source: "{#Root}\dist\windows\payload\*"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
[Icons]
Name: "{group}\Open Transfer Manager"; Filename: "{app}\otm.exe"; IconFilename: "{app}\otm.ico"
Name: "{group}\Browser extensions"; Filename: "{app}\extensions"
Name: "{group}\Installer extension Chrome"; Filename: "{app}\otm.exe"; Parameters: "--install-chrome-extension"; IconFilename: "{app}\otm.ico"
Name: "{group}\Desinstaller Open Transfer Manager"; Filename: "{app}\Desinstaller OTM.exe"; IconFilename: "{app}\otm.ico"
Name: "{autodesktop}\Open Transfer Manager"; Filename: "{app}\otm.exe"; IconFilename: "{app}\otm.ico"; Tasks: desktopicon
[Run]
Filename: "{app}\otm.exe"; Parameters: "register-browsers --force"; Flags: runhidden waituntilterminated
#ifdef ChromeStoreId
Filename: "{app}\otm.exe"; Parameters: "register-browsers --machine"; Flags: runhidden waituntilterminated
#endif
Filename: "{app}\otm.exe"; Parameters: "--install-chrome-extension"; Description: "Installer / configurer l’extension Chrome"; Flags: nowait postinstall skipifsilent
[UninstallRun]
Filename: "{app}\otm.exe"; Parameters: "unregister-browsers"; Flags: runhidden waituntilterminated; RunOnceId: "RemoveBrowserBridge"
#ifdef ChromeStoreId
Filename: "{app}\otm.exe"; Parameters: "unregister-browsers --machine"; Flags: runhidden waituntilterminated; RunOnceId: "RemoveMachineBrowserBridge"
[Registry]
Root: HKLM32; Subkey: "Software\Google\Chrome\Extensions\{#ChromeStoreId}"; ValueType: string; ValueName: "update_url"; ValueData: "https://clients2.google.com/service/update2/crx"; Flags: uninsdeletekey
#endif
