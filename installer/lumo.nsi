; Lumo installer (NSIS 3). Per-user install, no admin rights needed.
;
; The application name, install folder, startup value and executable use Lumo.
; Legacy names appear only in upgrade cleanup.
;
; Build (Linux/macOS/Windows with makensis):
;   makensis -DEXE=/abs/path/lumo.exe -DOUTDIR=/abs/path/dist installer/lumo.nsi
; tools/package.sh does this for you.

; Native 64-bit installer (Lumo itself is x64-only).
Target amd64-unicode
SetCompressor /SOLID lzma

!define APPNAME "Lumo"
!define VERSION "2026.1107.2"
!define PUBLISHER "mthlpbs"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Lumo"
!define RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"

!ifndef EXE
  !error "Pass -DEXE=<path to lumo.exe>"
!endif
!ifndef OUTDIR
  !define OUTDIR "."
!endif

Name "${APPNAME}"
OutFile "${OUTDIR}/Lumo-Setup-${VERSION}.exe"
RequestExecutionLevel user
InstallDir "$LOCALAPPDATA\Programs\Lumo"
InstallDirRegKey HKCU "Software\Lumo" "InstallDir"
BrandingText "${APPNAME} ${VERSION}"

VIProductVersion "2026.1107.2.0"
VIAddVersionKey "ProductName" "${APPNAME}"
VIAddVersionKey "CompanyName" "${PUBLISHER}"
VIAddVersionKey "FileDescription" "Lumo installer"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "Copyright (C) 2026 ${PUBLISHER}"

!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "WinMessages.nsh"
!include "FileFunc.nsh"
!include "Sections.nsh"

; Ask a running Lumo to quit (it saves its data on WM_CLOSE), wait up to 3 s,
; then force-close with the fully qualified system taskkill.
!macro StopLumo
  ; Older installations must save and quit before the renamed app starts.
  FindWindow $2 "Poni.Island"
  ${If} $2 <> 0
    SendMessage $2 ${WM_CLOSE} 0 0 /TIMEOUT=2000
  ${EndIf}
  FindWindow $0 "Lumo.Island"
  ${If} $0 <> 0
    SendMessage $0 ${WM_CLOSE} 0 0 /TIMEOUT=2000
    StrCpy $1 0
    ${DoWhile} $0 <> 0
      ${If} $1 >= 30
        ${Break}
      ${EndIf}
      Sleep 100
      FindWindow $0 "Lumo.Island"
      IntOp $1 $1 + 1
    ${Loop}
  ${EndIf}
  nsExec::Exec '"$SYSDIR\taskkill.exe" /IM lumo.exe /F'
  nsExec::Exec '"$SYSDIR\taskkill.exe" /IM poni.exe /F'
  Sleep 200
!macroend
!define MUI_ICON "..\crates\lumo\res\lumo.ico"
!define MUI_UNICON "..\crates\lumo\res\lumo.ico"
; Supplied Lumo logo on the welcome and header pages (tools/make_brand.py).
!define MUI_WELCOMEFINISHPAGE_BITMAP "art\nsis-welcome.bmp"
!define MUI_HEADERIMAGE
!define MUI_HEADERIMAGE_RIGHT
!define MUI_HEADERIMAGE_BITMAP "art\nsis-header.bmp"
!define MUI_ABORTWARNING
!define MUI_COMPONENTSPAGE_SMALLDESC
!define MUI_FINISHPAGE_RUN "$INSTDIR\lumo.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Launch Lumo now"
!define MUI_WELCOMEPAGE_TITLE "Welcome to Lumo"
!define MUI_WELCOMEPAGE_TEXT "Install Lumo for your Windows account.$\r$\n$\r$\nAdministrator rights are not required."

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Section "Lumo" SecMain
  SectionIn RO
  ; Close a running copy so the exe can be replaced.
  !insertmacro StopLumo
  SetOutPath "$INSTDIR"
  File "/oname=lumo.exe" "${EXE}"
  ; The licenses of what lumo.exe is built from (mecab-ipadic asks for its notice in every copy).
  File "..\THIRD-PARTY-NOTICES.txt"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  ; Remove obsolete shortcuts when upgrading to the new brand.
  Delete "$SMPROGRAMS\Poni.lnk"
  Delete "$SMPROGRAMS\Oyen Poni.lnk"
  Delete "$SMPROGRAMS\Oyenza Studio.lnk"
  CreateShortcut "$SMPROGRAMS\${APPNAME}.lnk" "$INSTDIR\lumo.exe" "" "$INSTDIR\lumo.exe" 0
  WriteRegStr HKCU "Software\Lumo" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "${APPNAME}"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "${PUBLISHER}"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\lumo.exe"
  WriteRegStr HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegStr HKCU "${UNINST_KEY}" "QuietUninstallString" '"$INSTDIR\Uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "EstimatedSize" 7680
SectionEnd

Section "Start Lumo with Windows" SecAutostart
  ; Same value the app's own "Start with Windows" toggle writes.
  WriteRegStr HKCU "${RUN_KEY}" "Lumo" '"$INSTDIR\lumo.exe"'
  DeleteRegValue HKCU "${RUN_KEY}" "Poni"
SectionEnd

; The app's own update runs this Setup silently: `/S /reopen` (crates/lumo/src/services/update.rs).
Function .onInit
  ; Silent: Start with Windows stays as the person set it (the Run value already points here).
  ${If} ${Silent}
    !insertmacro UnselectSection ${SecAutostart}
  ${EndIf}
FunctionEnd

Function .onInstSuccess
  ; The finish page's "Launch Lumo now" never shows in silent mode: /reopen starts it.
  ${If} ${Silent}
    ${GetParameters} $R0
    ClearErrors
    ${GetOptions} $R0 "/reopen" $R1
    ${IfNot} ${Errors}
      Exec '"$INSTDIR\lumo.exe"'
    ${EndIf}
  ${EndIf}
FunctionEnd

!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SecMain} "The Lumo app and a Start Menu shortcut."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecAutostart} "Open Lumo automatically when you sign in. You can change this later from its menu."
!insertmacro MUI_FUNCTION_DESCRIPTION_END

Section "Uninstall"
  !insertmacro StopLumo
  Delete "$INSTDIR\lumo.exe"
  Delete "$INSTDIR\THIRD-PARTY-NOTICES.txt"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\${APPNAME}.lnk"
  Delete "$SMPROGRAMS\Lumo.lnk"
  Delete "$SMPROGRAMS\Lumo.lnk"
  Delete "$SMPROGRAMS\Lumo.lnk"
  DeleteRegValue HKCU "${RUN_KEY}" "Lumo"
  DeleteRegKey HKCU "${UNINST_KEY}"
  DeleteRegKey HKCU "Software\Lumo"
  ; Cache and log (lyrics lookups name the songs you played) go too.
  RMDir /r "$LOCALAPPDATA\Lumo"
  ; Your settings and data (tasks, shelf, focus history) in %APPDATA%\Lumo are kept.
SectionEnd
