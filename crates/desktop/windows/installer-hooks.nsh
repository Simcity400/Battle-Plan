
!define HOI_RECORDER "the game Recorder"
!define HOI_RECORDER_EXE "battle-plan-recorder.exe"
!define HOI_APP_EXE "replay-desktop.exe"
!define HOI_RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"

Var HoiStartCheckbox
Var HoiStartRecorder
Var HoiRunning
Var HoiAppClosed
Var HoiRecorderRan
Var HoiReplayRan
Var HoiGuard
Var HoiClosing
Var HoiStateFile

!define HOI_GUARD "Local\HOI4Replay.Setup"
!define HOI_STATE "HOI4REPLAY_SETUP_STATE"

!macro HOI_TAKE_GUARD
  System::Call 'kernel32::CreateMutexW(p 0, i 1, w "${HOI_GUARD}") p .r0 ?e'
  Pop $1
  ${If} $0 == 0
    MessageBox MB_ICONEXCLAMATION|MB_OK "Setup could not claim Battle Plan's install (error $1), so it stopped." /SD IDOK
    Abort
  ${EndIf}
  ${If} $1 = 183
    System::Call 'kernel32::CloseHandle(p r0)'
    MessageBox MB_ICONEXCLAMATION|MB_OK "Another Battle Plan setup is running. Let it finish, then run this one again." /SD IDOK
    Abort
  ${EndIf}
  StrCpy $HoiGuard $0
!macroend

!macro HOI_RELEASE_GUARD
  ${If} $HoiGuard != ""
    System::Call 'kernel32::CloseHandle(p $HoiGuard)'
    StrCpy $HoiGuard ""
  ${EndIf}
!macroend

!macro HOI_READ_STATE
  ${If} $HoiStateFile != ""
    ReadINIStr $0 "$HoiStateFile" "closed" "recorder"
    ${IfThen} $0 == 1 ${|} StrCpy $HoiRecorderRan 1 ${|}
    ReadINIStr $0 "$HoiStateFile" "closed" "app"
    ${IfThen} $0 == 1 ${|} StrCpy $HoiReplayRan 1 ${|}
    Delete "$HoiStateFile"
  ${EndIf}
!macroend

!macro HOI_RUN_AS_USER program arguments
  Push "${arguments}"
  Push "${program}"
  CallInstDLL "$PLUGINSDIR\nsis_tauri_utils.dll" RunAsUser
!macroend

Function .onInstFailed
  ${If} $HoiClosing != 1
    Return
  ${EndIf}
  !insertmacro HOI_RELEASE_GUARD
  !insertmacro HOI_READ_STATE
  ${GetOptions} $CMDLINE "/HOI4RECORDER" $0
  ${IfNot} ${Errors}
    StrCpy $HoiRecorderRan 1
  ${EndIf}
  ${GetOptions} $CMDLINE "/R" $0
  ${IfNot} ${Errors}
    StrCpy $HoiReplayRan 1
  ${EndIf}
  ${IfNot} ${FileExists} "$INSTDIR\${HOI_RECORDER_EXE}"
    Return
  ${EndIf}
  ${If} $HoiRecorderRan == 1
    !insertmacro HOI_RUN_AS_USER "$INSTDIR\${HOI_RECORDER_EXE}" ""
  ${EndIf}
  ${If} $HoiReplayRan == 1
    !insertmacro HOI_RUN_AS_USER "$INSTDIR\${HOI_APP_EXE}" ""
  ${EndIf}
FunctionEnd

Function un.onUninstFailed
  ${If} $HoiClosing != 1
    Return
  ${EndIf}
  !insertmacro HOI_RELEASE_GUARD
  ${If} $HoiStateFile != ""
    DeleteINIStr "$HoiStateFile" "closed" "recorder"
    DeleteINIStr "$HoiStateFile" "closed" "app"
  ${EndIf}
  ${IfNot} ${FileExists} "$INSTDIR\${HOI_RECORDER_EXE}"
    Return
  ${EndIf}
  ${If} $HoiRecorderRan == 1
    !insertmacro HOI_RUN_AS_USER "$INSTDIR\${HOI_RECORDER_EXE}" ""
  ${EndIf}
  ${If} $HoiAppClosed == 1
    !insertmacro HOI_RUN_AS_USER "$INSTDIR\${HOI_APP_EXE}" ""
  ${EndIf}
FunctionEnd

!define MUI_CUSTOMFUNCTION_GUIINIT HoiGuiInit
Function HoiGuiInit
  System::Call 'kernel32::GetCurrentProcessId() i .r1'
  StrCpy $HoiStateFile "$TEMP\HOI4Replay-setup-$1.ini"
  System::Call 'kernel32::SetEnvironmentVariableW(w "${HOI_STATE}", w "$HoiStateFile") i .r2'
FunctionEnd

!define MUI_CUSTOMFUNCTION_ABORT HoiAbort
Function HoiAbort
  ${If} $HoiStateFile != ""
    Delete "$HoiStateFile"
  ${EndIf}
FunctionEnd

!define MUI_WELCOMEPAGE_TEXT "This installs Battle Plan and Battle Plan Recorder for your Windows account. No administrator rights are needed.$\r$\n$\r$\nBattle Plan Recorder records your campaigns while the game runs.$\r$\n$\r$\n$_CLICK"
!define MUI_PAGE_CUSTOMFUNCTION_SHOW HoiWelcomeShow
!define MUI_PAGE_CUSTOMFUNCTION_LEAVE HoiWelcomeLeave

Function HoiWelcomeShow
  ${NSD_CreateCheckbox} 120u 172u 195u 12u "Start Battle Plan Recorder when Windows starts"
  Pop $HoiStartCheckbox
  SetCtlColors $HoiStartCheckbox "" "${MUI_BGCOLOR}"
  ReadRegStr $0 HKCU "${HOI_RUN_KEY}" "${HOI_RECORDER}"
  ${If} $0 != ""
  ${OrIfNot} ${FileExists} "$INSTDIR\${HOI_RECORDER_EXE}"
    ${NSD_Check} $HoiStartCheckbox
  ${EndIf}
FunctionEnd

Function HoiWelcomeLeave
  ${NSD_GetState} $HoiStartCheckbox $HoiStartRecorder
FunctionEnd

!macro HOI_CLOSE role exe name
  StrCpy $HoiRunning 0
  StrCpy $HoiAppClosed 0
  nsis_tauri_utils::FindProcessCurrentUser "${exe}"
  Pop $0
  ${If} $0 = 0
    StrCpy $HoiRunning 1
    ${If} $PassiveMode <> 1
    ${AndIfNot} ${Silent}
      MessageBox MB_ICONINFORMATION|MB_OKCANCEL "${name} is running. Setup closes it to continue." /SD IDOK IDOK +2
      Abort
    ${EndIf}
    System::Call 'kernel32::GetCurrentProcessId() i .r1'
    System::Call 'kernel32::ProcessIdToSessionId(i r1, *i .r2) i .r3'
    StrCpy $4 "\\.\pipe\HOI4Replay.$2.${role}"
    StrCpy $5 0
    ${Do}
      nsis_tauri_utils::FindProcessCurrentUser "${exe}"
      Pop $0
      ${IfThen} $0 <> 0 ${|} ${ExitDo} ${|}
      IntOp $6 $5 % 8
      ${If} $6 = 0
        System::Call 'kernel32::CallNamedPipeW(w r4, *i 67, i 1, *i 0 .r7, i 4, *i 0 .r8, i 60000) i .r9'
        IntOp $7 $7 & 0xFF
        ${If} $9 <> 0
        ${AndIf} $7 = 89
          StrCpy $HoiAppClosed 1
        ${EndIf}
        ${If} $9 <> 0
        ${AndIf} $7 = 78
          MessageBox MB_ICONEXCLAMATION|MB_OK "${name} is recording, saving a recording or attached to the game, so setup stopped. Run it again once ${name} is idle." /SD IDOK
          Abort
        ${EndIf}
      ${EndIf}
      Sleep 500
      IntOp $5 $5 + 1
      ${If} $5 > 240
        MessageBox MB_ICONEXCLAMATION|MB_OK "${name} did not close, so setup stopped. Close it, then run setup again." /SD IDOK
        Abort
      ${EndIf}
    ${Loop}
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro HOI_TAKE_GUARD
  StrCpy $HoiClosing 1
  !insertmacro HOI_READ_STATE
  !insertmacro HOI_CLOSE "recorder" "${HOI_RECORDER_EXE}" "${HOI_RECORDER}"
  ${IfThen} $HoiRunning == 1 ${|} StrCpy $HoiRecorderRan 1 ${|}
  !insertmacro HOI_CLOSE "dashboard" "${MAINBINARYNAME}.exe" "${PRODUCTNAME}"
  ${IfThen} $HoiAppClosed == 1 ${|} StrCpy $HoiReplayRan 1 ${|}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  Delete "$INSTDIR\viewer.exe"
  RMDir /r "$INSTDIR\shader-tools"
  RMDir /r "$INSTDIR\LICENSES"
  ${If} $UpdateMode <> 1
  ${AndIf} $NoShortcutMode <> 1
    CreateShortcut "$SMPROGRAMS\${HOI_RECORDER}.lnk" "$INSTDIR\${HOI_RECORDER_EXE}"
  ${EndIf}
  ${If} $HoiStartRecorder == 1
    WriteRegStr HKCU "${HOI_RUN_KEY}" "${HOI_RECORDER}" '"$INSTDIR\${HOI_RECORDER_EXE}"'
  ${ElseIf} $HoiStartRecorder == 0
    DeleteRegValue HKCU "${HOI_RUN_KEY}" "${HOI_RECORDER}"
  ${EndIf}
  !insertmacro HOI_RELEASE_GUARD
  StrCpy $HoiClosing 0
  ${GetOptions} $CMDLINE "/HOI4RECORDER" $0
  ${IfNot} ${Errors}
    StrCpy $HoiRecorderRan 1
  ${EndIf}
  ${If} $HoiRecorderRan == 1
    nsis_tauri_utils::RunAsUser "$INSTDIR\${HOI_RECORDER_EXE}" ""
  ${EndIf}
  ${If} $HoiReplayRan == 1
  ${AndIf} $PassiveMode = 1
    ${GetOptions} $CMDLINE "/R" $0
    ${If} ${Errors}
      nsis_tauri_utils::RunAsUser "$INSTDIR\${MAINBINARYNAME}.exe" ""
    ${EndIf}
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ReadEnvStr $HoiStateFile "${HOI_STATE}"
  !insertmacro HOI_TAKE_GUARD
  StrCpy $HoiClosing 1
  !insertmacro HOI_CLOSE "recorder" "${HOI_RECORDER_EXE}" "${HOI_RECORDER}"
  StrCpy $HoiRecorderRan $HoiRunning
  ${If} $HoiRunning == 1
  ${AndIf} $HoiStateFile != ""
    WriteINIStr "$HoiStateFile" "closed" "recorder" "1"
  ${EndIf}
  !insertmacro HOI_CLOSE "dashboard" "${MAINBINARYNAME}.exe" "${PRODUCTNAME}"
  ${If} $HoiAppClosed == 1
  ${AndIf} $HoiStateFile != ""
    WriteINIStr "$HoiStateFile" "closed" "app" "1"
  ${EndIf}
  RMDir /r "$INSTDIR\LICENSES"
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
    Delete "$SMPROGRAMS\${HOI_RECORDER}.lnk"
    DeleteRegValue HKCU "${HOI_RUN_KEY}" "${HOI_RECORDER}"
  ${EndIf}
!macroend
