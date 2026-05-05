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

[Files]
Source: "release\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\RustTracker"; Filename: "{app}\rusttracker.exe"
Name: "{group}\Uninstall RustTracker"; Filename: "{uninstallexe}"
