; Per-user installer, built by .github/workflows/release.yml.
;
; It exists for one reason. The startup entry under HKCU\...\Run holds a full path, and the
; released exe carries its version in the file name, so that path stops naming anything the
; moment the next version arrives. Installing puts the exe somewhere that does not move
; between versions, and the entry keeps working.
;
; The portable exe is still the main way to run this. Nothing here is required.

Unicode true

!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"

; The workflow passes all three. The defaults are for building from a working tree, so this
; can be tried locally without going near a release.
!ifndef VERSION
  !define VERSION "0.0.0"
!endif
; Four numbers and nothing else, which is what the version resource will take. The workflow
; derives it, because `0.1.0-rc.1` is a version Cargo is happy with and this field is not.
!ifndef PRODUCT_VERSION
  !define PRODUCT_VERSION "0.0.0.0"
!endif
!ifndef SOURCE_EXE
  !define SOURCE_EXE "..\target\release\inzone-h9-gen1-headset-status.exe"
!endif
!ifndef OUT_FILE
  !define OUT_FILE "..\dist\inzone-h9-gen1-headset-status-setup.exe"
!endif

!define APP "inzone-h9-gen1-headset-status"
!define EXE "${APP}.exe"
; The name `claim_single_instance` creates in main.rs. Opening it is how both halves below
; find a running copy; see the comment on EnsureNotRunning.
!define MUTEX "Local\${APP}"
!define RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP}"

Name "${APP}"
OutFile "${OUT_FILE}"
; Per user. This app needs nothing a plain account cannot do, and asking for elevation would
; put a UAC prompt in front of a tray utility that writes one file and one registry value.
RequestExecutionLevel user
InstallDir "$LOCALAPPDATA\Programs\${APP}"
; An upgrade goes back where the last one went, including a directory the user chose.
InstallDirRegKey HKCU "${UNINST_KEY}" "InstallLocation"
SetCompressor /SOLID lzma

; So the Properties tab of the downloaded setup.exe says which version it is. Without this
; the only way to tell two of them apart is the file name, which is the problem this whole
; file exists to solve.
VIProductVersion "${PRODUCT_VERSION}"
VIAddVersionKey "ProductName" "${APP}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "FileVersion" "${PRODUCT_VERSION}"
VIAddVersionKey "FileDescription" "${APP} installer"
VIAddVersionKey "LegalCopyright" ""

!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\${EXE}"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

; English first, so it is the fallback for every language this app does not speak. The same
; rule `Lang::from_windows` follows.
!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "Japanese"

LangString AlreadyRunning ${LANG_ENGLISH} "${APP} is running. Quit it from its tray icon, then run this again."
LangString AlreadyRunning ${LANG_JAPANESE} "${APP} が起動しています。タスクトレイのアイコンから終了してから、もう一度実行してください。"

; A running exe cannot be replaced or deleted, so both halves stop before touching anything.
;
; Through the mutex rather than the process name, because the name is the very thing that
; moves: the download is `inzone-h9-gen1-headset-status-v0.1.0.exe` and the installed copy is
; `inzone-h9-gen1-headset-status.exe`, so a name match finds one and misses the other. The
; mutex is the same either way. Measured both ways round, and against a name that exists
; nowhere.
;
; 0x00100000 is SYNCHRONIZE, the least this needs to ask for.
!macro EnsureNotRunning
  System::Call 'kernel32::OpenMutexW(i 0x100000, i 0, w "${MUTEX}") i .r0'
  ${If} $0 <> 0
    System::Call 'kernel32::CloseHandle(i $0)'
    MessageBox MB_OK|MB_ICONEXCLAMATION "$(AlreadyRunning)" /SD IDOK
    Abort
  ${EndIf}
!macroend

Function .onInit
  !insertmacro EnsureNotRunning
FunctionEnd

Function un.onInit
  !insertmacro EnsureNotRunning
FunctionEnd

Section "Install"
  SetOutPath "$INSTDIR"
  ; Renamed on the way in. The version lives in the download's name and in `--version`, and
  ; a path with a version in it is what this whole file exists to avoid.
  File /oname=${EXE} "${SOURCE_EXE}"
  ; The way to start it by hand, for anyone who leaves startup switched off.
  CreateShortcut "$SMPROGRAMS\${APP}.lnk" "$INSTDIR\${EXE}"
  WriteUninstaller "$INSTDIR\uninstall.exe"

  ; Deliberately no "start with Windows" here. The tray menu owns that switch and says so
  ; when the registry refuses it. A checkbox in an installer re-asserts itself at every
  ; upgrade, so someone who turned startup off would find it back on after an update they ran
  ; for an unrelated reason.

  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "${APP}"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "ugai"
  WriteRegStr HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\${EXE}"
  WriteRegStr HKCU "${UNINST_KEY}" "URLInfoAbout" "https://github.com/ugai/${APP}"
  WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "${UNINST_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "EstimatedSize" "$0"
SectionEnd

Section "Uninstall"
  ; Left behind, this names an exe that is about to stop existing, and Windows would go on
  ; trying to launch it at every logon without saying anything. That is the failure this
  ; installer was written for, so leaving a fresh one behind would be a poor joke.
  ;
  ; Only when it names the exe in this directory. A portable copy writes the same value under
  ; the same name, and removing the installed copy must not stop that one from starting.
  ; `StrCmp` is case-insensitive, which is the right comparison for a Windows path.
  ReadRegStr $0 HKCU "${RUN_KEY}" "${APP}"
  ${If} $0 == '"$INSTDIR\${EXE}"'
    DeleteRegValue HKCU "${RUN_KEY}" "${APP}"
  ${EndIf}

  Delete "$SMPROGRAMS\${APP}.lnk"
  Delete "$INSTDIR\${EXE}"
  Delete "$INSTDIR\uninstall.exe"
  ; Not /r. Anything else in there was put there by the user, and this has no business
  ; guessing what it was.
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "${UNINST_KEY}"

  ; settings.conf under %APPDATA% stays. It is small, it is the user's, and a reinstall
  ; picking up where they left off is the friendlier default. README says where it is.
SectionEnd
