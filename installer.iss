[Setup]
AppName=RustTracker
AppVersion=1.0.0
DefaultDirName={autopf}\RustTracker
ArchitecturesAllowed=x64
ArchitecturesInstallIn64BitMode=x64
DefaultGroupName=RustTracker
OutputBaseFilename=RustTracker-Setup
Compression=lzma2
SolidCompression=yes
OutputDir=setup
SetupIconFile=icon.ico
UninstallDisplayIcon={app}\rusttracker.exe

[Files]
Source: "release\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "icon.ico"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\RustTracker"; Filename: "{app}\rusttracker.exe"; IconFilename: "{app}\icon.ico"
Name: "{group}\Uninstall RustTracker"; Filename: "{uninstallexe}"; IconFilename: "{app}\icon.ico"
