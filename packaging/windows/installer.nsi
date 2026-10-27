; Ferrum installer. Build with: makensis /DVERSION=0.1.0 packaging\windows\installer.nsi
!include "MUI2.nsh"
Name "Ferrum"
OutFile "..\..\dist\Ferrum-${VERSION}-windows-x86_64-setup.exe"
InstallDir "$PROGRAMFILES64\Ferrum"
InstallDirRegKey HKLM "Software\Ferrum" "Install_Dir"
RequestExecutionLevel admin
!define MUI_ICON "..\icons\icon.ico"
!define MUI_UNICON "..\icons\icon.ico"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\ferrum.exe"
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_UNPAGE_FINISH
!insertmacro MUI_LANGUAGE "English"

Section "Ferrum"
  SetOutPath "$INSTDIR"
  File "..\..\target\release\ferrum.exe"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateDirectory "$SMPROGRAMS\Ferrum"
  CreateShortcut "$SMPROGRAMS\Ferrum\Ferrum.lnk" "$INSTDIR\ferrum.exe"
  CreateShortcut "$SMPROGRAMS\Ferrum\Uninstall.lnk" "$INSTDIR\Uninstall.exe"
  WriteRegStr HKLM "Software\Ferrum" "Install_Dir" "$INSTDIR"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Ferrum" "DisplayName" "Ferrum"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Ferrum" "DisplayVersion" "${VERSION}"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Ferrum" "DisplayIcon" "$INSTDIR\ferrum.exe"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Ferrum" "UninstallString" '"$INSTDIR\Uninstall.exe"'
SectionEnd

Section "Uninstall"
  Delete "$INSTDIR\ferrum.exe"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\Ferrum\*.lnk"
  RMDir "$SMPROGRAMS\Ferrum"
  DeleteRegKey HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Ferrum"
  DeleteRegKey HKLM "Software\Ferrum"
SectionEnd
