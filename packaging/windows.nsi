Unicode true
!include "MUI2.nsh"
Name "Forge"
OutFile "${OUTPUT}"
InstallDir "$LOCALAPPDATA\Programs\Forge"
InstallDirRegKey HKCU "Software\Forge" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma
!define MUI_ABORTWARNING
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "${PAYLOAD}\LICENSE"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"
Section "Forge"
  SetShellVarContext current
  SetOutPath "$INSTDIR"
  File "${PAYLOAD}\forge.exe"
  File "${PAYLOAD}\*.dll"
  File "${PAYLOAD}\LICENSE"
  File "${PAYLOAD}\THIRD_PARTY.md"
  File "${PAYLOAD}\README.txt"
  File /r "${PAYLOAD}\licenses"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateDirectory "$SMPROGRAMS\Forge"
  CreateShortcut "$SMPROGRAMS\Forge\Forge.lnk" "$INSTDIR\forge.exe"
  WriteRegStr HKCU "Software\Forge" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Forge" "DisplayName" "Forge"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Forge" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Forge" "UninstallString" '$\"$INSTDIR\Uninstall.exe$\"'
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Forge" "NoModify" 1
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Forge" "NoRepair" 1
SectionEnd
Section "Uninstall"
  SetShellVarContext current
  Delete "$SMPROGRAMS\Forge\Forge.lnk"
  RMDir "$SMPROGRAMS\Forge"
  Delete "$INSTDIR\forge.exe"
  Delete "$INSTDIR\vcruntime140.dll"
  Delete "$INSTDIR\vcruntime140_1.dll"
  Delete "$INSTDIR\msvcp140.dll"
  Delete "$INSTDIR\msvcp140_1.dll"
  Delete "$INSTDIR\msvcp140_2.dll"
  Delete "$INSTDIR\msvcp140_atomic_wait.dll"
  Delete "$INSTDIR\msvcp140_codecvt_ids.dll"
  Delete "$INSTDIR\concrt140.dll"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\THIRD_PARTY.md"
  Delete "$INSTDIR\README.txt"
  Delete "$INSTDIR\licenses\OFL-JetBrainsMono.txt"
  Delete "$INSTDIR\licenses\OFL-SpaceGrotesk.txt"
  RMDir "$INSTDIR\licenses"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Forge"
  DeleteRegKey HKCU "Software\Forge"
SectionEnd
