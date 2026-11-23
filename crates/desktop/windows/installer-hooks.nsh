
!searchparse /noerrors "$%BATTLEPLAN_VERSION%" "" HOI_RELEASE "-nightly." HOI_NIGHTLY
!ifdef HOI_NIGHTLY
  !define HOI_CHANNEL "Nightly"
  !define HOI_OTHER_CHANNEL "Alpha"
!else
  !define HOI_CHANNEL "Alpha"
  !define HOI_OTHER_CHANNEL "Nightly"
!endif
!define HOI_APP "BattlePlan (${HOI_CHANNEL})"
!define HOI_OTHER_APP "BattlePlan (${HOI_OTHER_CHANNEL})"
!define HOI_RECORDER "BattlePlan Recorder (${HOI_CHANNEL})"
!define HOI_OTHER_RECORDER "BattlePlan Recorder (${HOI_OTHER_CHANNEL})"
!define HOI_RUN_VALUE "BattlePlan Recorder"
!define HOI_LEGACY_RECORDER "the game Recorder"
!define HOI_LEGACY_APP "Battle Plan"
!define HOI_RECORDER_EXE "battleplan-recorder.exe"
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
Var HoiInstalled

!define HOI_GUARD "Local\BattlePlan.Setup"
!define HOI_STATE "BATTLEPLAN_SETUP_STATE"

!macro HOI_TAKE_GUARD
  System::Call 'kernel32::CreateMutexW(p 0, i 1, w "${HOI_GUARD}") p .r0 ?e'
  Pop $1
  ${If} $0 == 0
    MessageBox MB_ICONEXCLAMATION|MB_OK "Setup couldn’t claim BattlePlan’s install (error $1), so it stopped." /SD IDOK
    Abort
  ${EndIf}
  ${If} $1 = 183
    System::Call 'kernel32::CloseHandle(p r0)'
    MessageBox MB_ICONEXCLAMATION|MB_OK "Another BattlePlan setup is running. Let it finish, then run this one again." /SD IDOK
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
  ${GetOptions} $CMDLINE "/BATTLEPLANRECORDER" $0
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
  StrCpy $HoiStateFile "$TEMP\BattlePlan-setup-$1.ini"
  System::Call 'kernel32::SetEnvironmentVariableW(w "${HOI_STATE}", w "$HoiStateFile") i .r2'
FunctionEnd

!define MUI_CUSTOMFUNCTION_ABORT HoiAbort
Function HoiAbort
  ${If} $HoiStateFile != ""
    Delete "$HoiStateFile"
  ${EndIf}
FunctionEnd

!define MUI_WELCOMEPAGE_TEXT "This installs ${HOI_APP} and ${HOI_RECORDER} for your Windows account. No administrator rights are needed.$\r$\n$\r$\n${HOI_RECORDER} records your campaigns while the game runs.$\r$\n$\r$\n$_CLICK"
!define MUI_PAGE_CUSTOMFUNCTION_SHOW HoiWelcomeShow
!define MUI_PAGE_CUSTOMFUNCTION_LEAVE HoiWelcomeLeave

Function HoiWelcomeShow
  ${NSD_CreateCheckbox} 120u 172u 195u 12u "Start ${HOI_RECORDER} when Windows starts"
  Pop $HoiStartCheckbox
  SetCtlColors $HoiStartCheckbox "" "${MUI_BGCOLOR}"
  ReadRegStr $0 HKCU "${HOI_RUN_KEY}" "${HOI_RUN_VALUE}"
  ${If} $0 != ""
  ${OrIfNot} ${FileExists} "$INSTDIR\${HOI_RECORDER_EXE}"
    ${NSD_Check} $HoiStartCheckbox
  ${EndIf}
FunctionEnd

Function HoiWelcomeLeave
  ${NSD_GetState} $HoiStartCheckbox $HoiStartRecorder
FunctionEnd

!macro HOI_ASK_CLOSE
  StrCpy $6 ""
  StrCpy $3 0
  StrCpy $9 0
  System::Call 'kernel32::WaitNamedPipeW(w r4, i 60000) i .r8'
  System::Call 'kernel32::CreateFileW(w r4, i 0xC0000000, i 0, p 0, i 3, i 0, p 0) p .r7'
  ${If} $7 P<> -1
    System::Call 'kernel32::SetNamedPipeHandleState(p r7, *i 2, p 0, p 0) i .r8'
    System::Call 'kernel32::TransactNamedPipe(p r7, *i 67, i 1, *i .r3, i 4, *i .r8, p 0) i .r9'
    IntOp $3 $3 & 0xFF
    ${If} $9 <> 0
    ${AndIf} $3 = 78
      System::Alloc 2048
      Pop $1
      System::Call 'kernel32::ReadFile(p r7, p r1, i 2046, *i .r8, p 0) i .r2'
      ${If} $2 <> 0
        System::Call '*$1(&w1023 .r6)'
      ${EndIf}
      System::Free $1
    ${EndIf}
    System::Call 'kernel32::CloseHandle(p r7)'
  ${EndIf}
!macroend

!macro HOI_CLOSE role exe name busy
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
    StrCpy $4 "\\.\pipe\BattlePlan.$2.${role}"
    StrCpy $5 0
    ${Do}
      nsis_tauri_utils::FindProcessCurrentUser "${exe}"
      Pop $0
      ${IfThen} $0 <> 0 ${|} ${ExitDo} ${|}
      IntOp $6 $5 % 8
      ${If} $6 = 0
        !insertmacro HOI_ASK_CLOSE
        ${If} $9 <> 0
        ${AndIf} $3 = 89
          StrCpy $HoiAppClosed 1
        ${EndIf}
        ${If} $9 <> 0
        ${AndIf} $3 = 78
          ${If} $6 == ""
            MessageBox MB_ICONEXCLAMATION|MB_OK "${busy}" /SD IDOK
          ${Else}
            MessageBox MB_ICONEXCLAMATION|MB_OK "$6$\r$\n$\r$\nSetup stopped. Run it again once ${name} can close." /SD IDOK
          ${EndIf}
          Abort
        ${EndIf}
      ${EndIf}
      Sleep 500
      IntOp $5 $5 + 1
      ${If} $5 > 240
        MessageBox MB_ICONEXCLAMATION|MB_OK "${name} didn’t close, so setup stopped. Close it, then run setup again." /SD IDOK
        Abort
      ${EndIf}
    ${Loop}
  ${EndIf}
!macroend

!macro HOI_REMOVE_LICENSES
  Delete "$INSTDIR\LICENSES\DirectXShaderCompiler-LICENSE-LLVM.txt"
  Delete "$INSTDIR\LICENSES\DirectXShaderCompiler-LICENSE-MIT.txt"
  Delete "$INSTDIR\LICENSES\DirectXShaderCompiler-LICENSE-MS.txt"
  Delete "$INSTDIR\LICENSES\Fontsource-cascadia-code-LICENSE.txt"
  Delete "$INSTDIR\LICENSES\Fontsource-inter-LICENSE.txt"
  Delete "$INSTDIR\LICENSES\Fontsource-roboto-LICENSE.txt"
  Delete "$INSTDIR\LICENSES\Fontsource-roboto-condensed-LICENSE.txt"
  Delete "$INSTDIR\LICENSES\Fontsource-source-serif-4-LICENSE.txt"
  Delete "$INSTDIR\LICENSES\IJG-libjpeg-README.txt"
  Delete "$INSTDIR\LICENSES\Lucide-LICENSE.txt"
  Delete "$INSTDIR\LICENSES\PhysFS-LICENSE.txt"
  Delete "$INSTDIR\LICENSES\SDL-LICENSE.txt"
  Delete "$INSTDIR\LICENSES\byteorder-lite-LICENSE-MIT.txt"
  Delete "$INSTDIR\LICENSES\eframe-LICENSE-APACHE.txt"
  Delete "$INSTDIR\LICENSES\eframe-LICENSE-MIT.txt"
  Delete "$INSTDIR\LICENSES\image-LICENSE-MIT.txt"
  Delete "$INSTDIR\LICENSES\libpng-LICENSE.txt"
  Delete "$INSTDIR\LICENSES\moxcms-LICENSE-BSD.txt"
  Delete "$INSTDIR\LICENSES\pxfm-LICENSE-BSD.txt"
  Delete "$INSTDIR\LICENSES\tao-LICENSE-APACHE.txt"
  Delete "$INSTDIR\LICENSES\tao-LICENSE.spdx"
  Delete "$INSTDIR\LICENSES\winit-LICENSE-APACHE.txt"
  Delete "$INSTDIR\LICENSES\xxhash-BSD-2-Clause.txt"
  RMDir "$INSTDIR\LICENSES"
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro HOI_TAKE_GUARD
  StrCpy $HoiClosing 1
  !insertmacro HOI_READ_STATE
  !insertmacro HOI_CLOSE "recorder" "${HOI_RECORDER_EXE}" "${HOI_RECORDER}" "${HOI_RECORDER} is recording, saving a recording or attached to the game, so setup stopped. Run it again once ${HOI_RECORDER} is idle."
  ${IfThen} $HoiRunning == 1 ${|} StrCpy $HoiRecorderRan 1 ${|}
  !insertmacro HOI_CLOSE "dashboard" "${MAINBINARYNAME}.exe" "${HOI_APP}" "${HOI_APP} has unsaved changes, so setup stopped. Save or discard them in ${HOI_APP}, then run setup again."
  ${IfThen} $HoiAppClosed == 1 ${|} StrCpy $HoiReplayRan 1 ${|}
!macroend

!macro HOI_REPLACE_APP_SHORTCUT folder old
  ${If} ${FileExists} "${folder}\${old}.lnk"
    CreateShortcut "${folder}\${HOI_APP}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    !insertmacro SetLnkAppUserModelId "${folder}\${HOI_APP}.lnk"
    Delete "${folder}\${old}.lnk"
  ${EndIf}
!macroend

!macro HOI_NAME_APP_SHORTCUTS folder
  !insertmacro HOI_REPLACE_APP_SHORTCUT "${folder}" "${HOI_LEGACY_APP}"
  !insertmacro HOI_REPLACE_APP_SHORTCUT "${folder}" "${PRODUCTNAME}"
  !insertmacro HOI_REPLACE_APP_SHORTCUT "${folder}" "${HOI_OTHER_APP}"
!macroend

!macro HOI_REPLACE_RECORDER_SHORTCUT old
  ${If} ${FileExists} "$SMPROGRAMS\${old}.lnk"
    CreateShortcut "$SMPROGRAMS\${HOI_RECORDER}.lnk" "$INSTDIR\${HOI_RECORDER_EXE}"
    Delete "$SMPROGRAMS\${old}.lnk"
  ${EndIf}
!macroend

Function .onGUIEnd
  ${If} $HoiInstalled == 1
  ${AndIf} ${FileExists} "$DESKTOP\$(^Name).lnk"
    Delete "$DESKTOP\${HOI_APP}.lnk"
    Rename "$DESKTOP\$(^Name).lnk" "$DESKTOP\${HOI_APP}.lnk"
  ${EndIf}
FunctionEnd

!macro NSIS_HOOK_POSTINSTALL
  !insertmacro HOI_NAME_APP_SHORTCUTS "$SMPROGRAMS"
  !insertmacro HOI_NAME_APP_SHORTCUTS "$DESKTOP"
  WriteRegStr SHCTX "${UNINSTKEY}" "DisplayName" "${HOI_APP}"
  Delete "$INSTDIR\viewer.exe"
  RMDir /r "$INSTDIR\shader-tools"
  !insertmacro HOI_REMOVE_LICENSES
  ${If} $UpdateMode <> 1
  ${AndIf} $NoShortcutMode <> 1
    CreateShortcut "$SMPROGRAMS\${HOI_RECORDER}.lnk" "$INSTDIR\${HOI_RECORDER_EXE}"
  ${EndIf}
  !insertmacro HOI_REPLACE_RECORDER_SHORTCUT "${HOI_LEGACY_RECORDER}"
  !insertmacro HOI_REPLACE_RECORDER_SHORTCUT "${HOI_LEGACY_APP} Recorder"
  !insertmacro HOI_REPLACE_RECORDER_SHORTCUT "${PRODUCTNAME} Recorder"
  !insertmacro HOI_REPLACE_RECORDER_SHORTCUT "${HOI_OTHER_RECORDER}"
  ${If} $HoiStartRecorder == 1
    WriteRegStr HKCU "${HOI_RUN_KEY}" "${HOI_RUN_VALUE}" '"$INSTDIR\${HOI_RECORDER_EXE}"'
  ${ElseIf} $HoiStartRecorder == 0
    DeleteRegValue HKCU "${HOI_RUN_KEY}" "${HOI_RUN_VALUE}"
  ${EndIf}
  !insertmacro HOI_RELEASE_GUARD
  StrCpy $HoiClosing 0
  StrCpy $HoiInstalled 1
  ${GetOptions} $CMDLINE "/BATTLEPLANRECORDER" $0
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
  !insertmacro HOI_CLOSE "recorder" "${HOI_RECORDER_EXE}" "${HOI_RECORDER}" "${HOI_RECORDER} is recording, saving a recording or attached to the game, so setup stopped. Run it again once ${HOI_RECORDER} is idle."
  StrCpy $HoiRecorderRan $HoiRunning
  ${If} $HoiRunning == 1
  ${AndIf} $HoiStateFile != ""
    WriteINIStr "$HoiStateFile" "closed" "recorder" "1"
  ${EndIf}
  !insertmacro HOI_CLOSE "dashboard" "${MAINBINARYNAME}.exe" "${HOI_APP}" "${HOI_APP} has unsaved changes, so setup stopped. Save or discard them in ${HOI_APP}, then run setup again."
  ${If} $HoiAppClosed == 1
  ${AndIf} $HoiStateFile != ""
    WriteINIStr "$HoiStateFile" "closed" "app" "1"
  ${EndIf}
  !insertmacro HOI_REMOVE_LICENSES
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
    !insertmacro UnpinShortcut "$SMPROGRAMS\${HOI_APP}.lnk"
    Delete "$SMPROGRAMS\${HOI_APP}.lnk"
    !insertmacro UnpinShortcut "$DESKTOP\${HOI_APP}.lnk"
    Delete "$DESKTOP\${HOI_APP}.lnk"
    Delete "$SMPROGRAMS\${HOI_RECORDER}.lnk"
    DeleteRegValue HKCU "${HOI_RUN_KEY}" "${HOI_RUN_VALUE}"
  ${EndIf}
!macroend
