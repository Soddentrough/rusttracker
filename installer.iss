[Setup]
AppName=RustTracker
AppVersion={#MyAppVersion}
DefaultDirName={autopf}\RustTracker
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
DefaultGroupName=RustTracker
OutputBaseFilename=RustTracker-Setup
Compression=lzma2
SolidCompression=yes
OutputDir=setup
SetupIconFile=icon.ico
UninstallDisplayIcon={app}\rusttracker.exe
ChangesAssociations=yes

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "assoc_tracker"; Description: "Associate Tracker Modules (.mod, .xm, .s3m, .it, .mptm, .stm, .669, .mtm, .med, .okt, .psm)"; GroupDescription: "File Associations:"; Flags: checkedonce
Name: "assoc_audio"; Description: "Associate Audio Files (.flac, .mp3, .wav, .ogg, .opus, .aac, .m4a, .mid, .midi, .aif, .wma)"; GroupDescription: "File Associations:"; Flags: unchecked
Name: "assoc_video"; Description: "Associate Video Containers (.mp4, .mkv, .webm, .avi, .mov)"; GroupDescription: "File Associations:"; Flags: unchecked
Name: "assoc_playlist"; Description: "Associate Playlists & DAW Files (.pls, .m3u, .m3u8, .dawproject, .aaf)"; GroupDescription: "File Associations:"; Flags: unchecked
Name: "contextmenu"; Description: "Add 'Play with RustTracker' to Explorer context menu"; GroupDescription: "Explorer Integration:"; Flags: checkedonce

[Files]
Source: "release\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "icon.ico"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\RustTracker"; Filename: "{app}\rusttracker.exe"; IconFilename: "{app}\icon.ico"
Name: "{group}\Uninstall RustTracker"; Filename: "{uninstallexe}"; IconFilename: "{app}\icon.ico"
Name: "{autodesktop}\RustTracker"; Filename: "{app}\rusttracker.exe"; IconFilename: "{app}\icon.ico"; Tasks: desktopicon

[Registry]
; Application Capabilities and Open With / SupportedTypes Registration
Root: HKA; Subkey: "Software\Classes\Applications\rusttracker.exe"; ValueType: string; ValueName: "FriendlyAppName"; ValueData: "RustTracker"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\rusttracker.exe\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\icon.ico,0"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\rusttracker.exe\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\rusttracker.exe"" ""%1"""; Flags: uninsdeletekey

; SupportedTypes entries
Root: HKA; Subkey: "Software\Classes\Applications\rusttracker.exe\SupportedTypes"; ValueType: string; ValueName: ".mod"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\rusttracker.exe\SupportedTypes"; ValueType: string; ValueName: ".xm"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\rusttracker.exe\SupportedTypes"; ValueType: string; ValueName: ".s3m"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\rusttracker.exe\SupportedTypes"; ValueType: string; ValueName: ".it"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\rusttracker.exe\SupportedTypes"; ValueType: string; ValueName: ".mptm"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\rusttracker.exe\SupportedTypes"; ValueType: string; ValueName: ".stm"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\rusttracker.exe\SupportedTypes"; ValueType: string; ValueName: ".669"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\rusttracker.exe\SupportedTypes"; ValueType: string; ValueName: ".mtm"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\rusttracker.exe\SupportedTypes"; ValueType: string; ValueName: ".med"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\rusttracker.exe\SupportedTypes"; ValueType: string; ValueName: ".okt"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\rusttracker.exe\SupportedTypes"; ValueType: string; ValueName: ".psm"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\rusttracker.exe\SupportedTypes"; ValueType: string; ValueName: ".flac"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\rusttracker.exe\SupportedTypes"; ValueType: string; ValueName: ".wav"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\rusttracker.exe\SupportedTypes"; ValueType: string; ValueName: ".mp3"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\rusttracker.exe\SupportedTypes"; ValueType: string; ValueName: ".ogg"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\rusttracker.exe\SupportedTypes"; ValueType: string; ValueName: ".opus"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\rusttracker.exe\SupportedTypes"; ValueType: string; ValueName: ".aac"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\rusttracker.exe\SupportedTypes"; ValueType: string; ValueName: ".m4a"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\rusttracker.exe\SupportedTypes"; ValueType: string; ValueName: ".mid"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\rusttracker.exe\SupportedTypes"; ValueType: string; ValueName: ".midi"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\rusttracker.exe\SupportedTypes"; ValueType: string; ValueName: ".aif"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\rusttracker.exe\SupportedTypes"; ValueType: string; ValueName: ".aiff"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\rusttracker.exe\SupportedTypes"; ValueType: string; ValueName: ".wma"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\rusttracker.exe\SupportedTypes"; ValueType: string; ValueName: ".mp4"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\rusttracker.exe\SupportedTypes"; ValueType: string; ValueName: ".mkv"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\rusttracker.exe\SupportedTypes"; ValueType: string; ValueName: ".webm"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\rusttracker.exe\SupportedTypes"; ValueType: string; ValueName: ".avi"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\rusttracker.exe\SupportedTypes"; ValueType: string; ValueName: ".mov"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\rusttracker.exe\SupportedTypes"; ValueType: string; ValueName: ".pls"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\rusttracker.exe\SupportedTypes"; ValueType: string; ValueName: ".m3u"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\rusttracker.exe\SupportedTypes"; ValueType: string; ValueName: ".m3u8"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\rusttracker.exe\SupportedTypes"; ValueType: string; ValueName: ".dawproject"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\rusttracker.exe\SupportedTypes"; ValueType: string; ValueName: ".aaf"; ValueData: ""; Flags: uninsdeletevalue

; ProgIDs Definitions
Root: HKA; Subkey: "Software\Classes\RustTracker.TrackerModule"; ValueType: string; ValueName: ""; ValueData: "Tracker Module"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\RustTracker.TrackerModule\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\icon.ico,0"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\RustTracker.TrackerModule\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\rusttracker.exe"" ""%1"""; Flags: uninsdeletekey

Root: HKA; Subkey: "Software\Classes\RustTracker.AudioFile"; ValueType: string; ValueName: ""; ValueData: "Audio File"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\RustTracker.AudioFile\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\icon.ico,0"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\RustTracker.AudioFile\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\rusttracker.exe"" ""%1"""; Flags: uninsdeletekey

Root: HKA; Subkey: "Software\Classes\RustTracker.VideoFile"; ValueType: string; ValueName: ""; ValueData: "Video File"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\RustTracker.VideoFile\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\icon.ico,0"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\RustTracker.VideoFile\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\rusttracker.exe"" ""%1"""; Flags: uninsdeletekey

Root: HKA; Subkey: "Software\Classes\RustTracker.PlaylistFile"; ValueType: string; ValueName: ""; ValueData: "Playlist / Project File"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\RustTracker.PlaylistFile\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\icon.ico,0"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\RustTracker.PlaylistFile\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\rusttracker.exe"" ""%1"""; Flags: uninsdeletekey

; File Association Mappings - Tracker Modules
Root: HKA; Subkey: "Software\Classes\.mod\OpenWithProgids"; ValueType: string; ValueName: "RustTracker.TrackerModule"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.mod"; ValueType: string; ValueName: ""; ValueData: "RustTracker.TrackerModule"; Flags: uninsdeletevalue; Tasks: assoc_tracker
Root: HKA; Subkey: "Software\Classes\.xm\OpenWithProgids"; ValueType: string; ValueName: "RustTracker.TrackerModule"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.xm"; ValueType: string; ValueName: ""; ValueData: "RustTracker.TrackerModule"; Flags: uninsdeletevalue; Tasks: assoc_tracker
Root: HKA; Subkey: "Software\Classes\.s3m\OpenWithProgids"; ValueType: string; ValueName: "RustTracker.TrackerModule"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.s3m"; ValueType: string; ValueName: ""; ValueData: "RustTracker.TrackerModule"; Flags: uninsdeletevalue; Tasks: assoc_tracker
Root: HKA; Subkey: "Software\Classes\.it\OpenWithProgids"; ValueType: string; ValueName: "RustTracker.TrackerModule"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.it"; ValueType: string; ValueName: ""; ValueData: "RustTracker.TrackerModule"; Flags: uninsdeletevalue; Tasks: assoc_tracker
Root: HKA; Subkey: "Software\Classes\.mptm\OpenWithProgids"; ValueType: string; ValueName: "RustTracker.TrackerModule"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.mptm"; ValueType: string; ValueName: ""; ValueData: "RustTracker.TrackerModule"; Flags: uninsdeletevalue; Tasks: assoc_tracker
Root: HKA; Subkey: "Software\Classes\.stm\OpenWithProgids"; ValueType: string; ValueName: "RustTracker.TrackerModule"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.stm"; ValueType: string; ValueName: ""; ValueData: "RustTracker.TrackerModule"; Flags: uninsdeletevalue; Tasks: assoc_tracker
Root: HKA; Subkey: "Software\Classes\.669\OpenWithProgids"; ValueType: string; ValueName: "RustTracker.TrackerModule"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.669"; ValueType: string; ValueName: ""; ValueData: "RustTracker.TrackerModule"; Flags: uninsdeletevalue; Tasks: assoc_tracker
Root: HKA; Subkey: "Software\Classes\.mtm\OpenWithProgids"; ValueType: string; ValueName: "RustTracker.TrackerModule"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.mtm"; ValueType: string; ValueName: ""; ValueData: "RustTracker.TrackerModule"; Flags: uninsdeletevalue; Tasks: assoc_tracker
Root: HKA; Subkey: "Software\Classes\.med\OpenWithProgids"; ValueType: string; ValueName: "RustTracker.TrackerModule"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.med"; ValueType: string; ValueName: ""; ValueData: "RustTracker.TrackerModule"; Flags: uninsdeletevalue; Tasks: assoc_tracker
Root: HKA; Subkey: "Software\Classes\.okt\OpenWithProgids"; ValueType: string; ValueName: "RustTracker.TrackerModule"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.okt"; ValueType: string; ValueName: ""; ValueData: "RustTracker.TrackerModule"; Flags: uninsdeletevalue; Tasks: assoc_tracker
Root: HKA; Subkey: "Software\Classes\.psm\OpenWithProgids"; ValueType: string; ValueName: "RustTracker.TrackerModule"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.psm"; ValueType: string; ValueName: ""; ValueData: "RustTracker.TrackerModule"; Flags: uninsdeletevalue; Tasks: assoc_tracker

; File Association Mappings - Audio Files
Root: HKA; Subkey: "Software\Classes\.flac\OpenWithProgids"; ValueType: string; ValueName: "RustTracker.AudioFile"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.flac"; ValueType: string; ValueName: ""; ValueData: "RustTracker.AudioFile"; Flags: uninsdeletevalue; Tasks: assoc_audio
Root: HKA; Subkey: "Software\Classes\.wav\OpenWithProgids"; ValueType: string; ValueName: "RustTracker.AudioFile"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.wav"; ValueType: string; ValueName: ""; ValueData: "RustTracker.AudioFile"; Flags: uninsdeletevalue; Tasks: assoc_audio
Root: HKA; Subkey: "Software\Classes\.mp3\OpenWithProgids"; ValueType: string; ValueName: "RustTracker.AudioFile"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.mp3"; ValueType: string; ValueName: ""; ValueData: "RustTracker.AudioFile"; Flags: uninsdeletevalue; Tasks: assoc_audio
Root: HKA; Subkey: "Software\Classes\.ogg\OpenWithProgids"; ValueType: string; ValueName: "RustTracker.AudioFile"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.ogg"; ValueType: string; ValueName: ""; ValueData: "RustTracker.AudioFile"; Flags: uninsdeletevalue; Tasks: assoc_audio
Root: HKA; Subkey: "Software\Classes\.opus\OpenWithProgids"; ValueType: string; ValueName: "RustTracker.AudioFile"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.opus"; ValueType: string; ValueName: ""; ValueData: "RustTracker.AudioFile"; Flags: uninsdeletevalue; Tasks: assoc_audio
Root: HKA; Subkey: "Software\Classes\.aac\OpenWithProgids"; ValueType: string; ValueName: "RustTracker.AudioFile"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.aac"; ValueType: string; ValueName: ""; ValueData: "RustTracker.AudioFile"; Flags: uninsdeletevalue; Tasks: assoc_audio
Root: HKA; Subkey: "Software\Classes\.m4a\OpenWithProgids"; ValueType: string; ValueName: "RustTracker.AudioFile"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.m4a"; ValueType: string; ValueName: ""; ValueData: "RustTracker.AudioFile"; Flags: uninsdeletevalue; Tasks: assoc_audio
Root: HKA; Subkey: "Software\Classes\.mid\OpenWithProgids"; ValueType: string; ValueName: "RustTracker.AudioFile"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.mid"; ValueType: string; ValueName: ""; ValueData: "RustTracker.AudioFile"; Flags: uninsdeletevalue; Tasks: assoc_audio
Root: HKA; Subkey: "Software\Classes\.midi\OpenWithProgids"; ValueType: string; ValueName: "RustTracker.AudioFile"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.midi"; ValueType: string; ValueName: ""; ValueData: "RustTracker.AudioFile"; Flags: uninsdeletevalue; Tasks: assoc_audio
Root: HKA; Subkey: "Software\Classes\.aif\OpenWithProgids"; ValueType: string; ValueName: "RustTracker.AudioFile"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.aif"; ValueType: string; ValueName: ""; ValueData: "RustTracker.AudioFile"; Flags: uninsdeletevalue; Tasks: assoc_audio
Root: HKA; Subkey: "Software\Classes\.aiff\OpenWithProgids"; ValueType: string; ValueName: "RustTracker.AudioFile"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.aiff"; ValueType: string; ValueName: ""; ValueData: "RustTracker.AudioFile"; Flags: uninsdeletevalue; Tasks: assoc_audio
Root: HKA; Subkey: "Software\Classes\.wma\OpenWithProgids"; ValueType: string; ValueName: "RustTracker.AudioFile"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.wma"; ValueType: string; ValueName: ""; ValueData: "RustTracker.AudioFile"; Flags: uninsdeletevalue; Tasks: assoc_audio

; File Association Mappings - Video Files
Root: HKA; Subkey: "Software\Classes\.mp4\OpenWithProgids"; ValueType: string; ValueName: "RustTracker.VideoFile"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.mp4"; ValueType: string; ValueName: ""; ValueData: "RustTracker.VideoFile"; Flags: uninsdeletevalue; Tasks: assoc_video
Root: HKA; Subkey: "Software\Classes\.mkv\OpenWithProgids"; ValueType: string; ValueName: "RustTracker.VideoFile"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.mkv"; ValueType: string; ValueName: ""; ValueData: "RustTracker.VideoFile"; Flags: uninsdeletevalue; Tasks: assoc_video
Root: HKA; Subkey: "Software\Classes\.webm\OpenWithProgids"; ValueType: string; ValueName: "RustTracker.VideoFile"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.webm"; ValueType: string; ValueName: ""; ValueData: "RustTracker.VideoFile"; Flags: uninsdeletevalue; Tasks: assoc_video
Root: HKA; Subkey: "Software\Classes\.avi\OpenWithProgids"; ValueType: string; ValueName: "RustTracker.VideoFile"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.avi"; ValueType: string; ValueName: ""; ValueData: "RustTracker.VideoFile"; Flags: uninsdeletevalue; Tasks: assoc_video
Root: HKA; Subkey: "Software\Classes\.mov\OpenWithProgids"; ValueType: string; ValueName: "RustTracker.VideoFile"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.mov"; ValueType: string; ValueName: ""; ValueData: "RustTracker.VideoFile"; Flags: uninsdeletevalue; Tasks: assoc_video

; File Association Mappings - Playlists & Projects
Root: HKA; Subkey: "Software\Classes\.pls\OpenWithProgids"; ValueType: string; ValueName: "RustTracker.PlaylistFile"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.pls"; ValueType: string; ValueName: ""; ValueData: "RustTracker.PlaylistFile"; Flags: uninsdeletevalue; Tasks: assoc_playlist
Root: HKA; Subkey: "Software\Classes\.m3u\OpenWithProgids"; ValueType: string; ValueName: "RustTracker.PlaylistFile"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.m3u"; ValueType: string; ValueName: ""; ValueData: "RustTracker.PlaylistFile"; Flags: uninsdeletevalue; Tasks: assoc_playlist
Root: HKA; Subkey: "Software\Classes\.m3u8\OpenWithProgids"; ValueType: string; ValueName: "RustTracker.PlaylistFile"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.m3u8"; ValueType: string; ValueName: ""; ValueData: "RustTracker.PlaylistFile"; Flags: uninsdeletevalue; Tasks: assoc_playlist
Root: HKA; Subkey: "Software\Classes\.dawproject\OpenWithProgids"; ValueType: string; ValueName: "RustTracker.PlaylistFile"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.dawproject"; ValueType: string; ValueName: ""; ValueData: "RustTracker.PlaylistFile"; Flags: uninsdeletevalue; Tasks: assoc_playlist
Root: HKA; Subkey: "Software\Classes\.aaf\OpenWithProgids"; ValueType: string; ValueName: "RustTracker.PlaylistFile"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\.aaf"; ValueType: string; ValueName: ""; ValueData: "RustTracker.PlaylistFile"; Flags: uninsdeletevalue; Tasks: assoc_playlist

; Context Menu Integration ("Play with RustTracker")
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\audio\shell\RustTracker"; ValueType: string; ValueName: ""; ValueData: "Play with RustTracker"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\audio\shell\RustTracker"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\icon.ico,0"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\audio\shell\RustTracker\command"; ValueType: string; ValueName: ""; ValueData: """{app}\rusttracker.exe"" ""%1"""; Flags: uninsdeletekey; Tasks: contextmenu

Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\video\shell\RustTracker"; ValueType: string; ValueName: ""; ValueData: "Play with RustTracker"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\video\shell\RustTracker"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\icon.ico,0"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\SystemFileAssociations\video\shell\RustTracker\command"; ValueType: string; ValueName: ""; ValueData: """{app}\rusttracker.exe"" ""%1"""; Flags: uninsdeletekey; Tasks: contextmenu

Root: HKA; Subkey: "Software\Classes\Directory\shell\RustTracker"; ValueType: string; ValueName: ""; ValueData: "Play with RustTracker"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\Directory\shell\RustTracker"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\icon.ico,0"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\Directory\shell\RustTracker\command"; ValueType: string; ValueName: ""; ValueData: """{app}\rusttracker.exe"" ""%1"""; Flags: uninsdeletekey; Tasks: contextmenu
