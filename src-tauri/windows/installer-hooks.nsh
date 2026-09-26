; agent-plugins.com is the console twin of agent-plugins.exe. For a bare
; `agent-plugins`, shells run .com before .exe and wait for it, so commands
; print their output and return their exit code. makensis runs beside the
; generated installer.nsi, two folders below the build output.
!macro NSIS_HOOK_POSTINSTALL
  File "/oname=$INSTDIR\agent-plugins.com" "..\..\agent-plugins-console.exe"
!macroend

; The app puts its folder on the user PATH. Take it off while the app is still
; here to do it, unless an update is replacing this copy. Should the uninstall
; stop short, the app puts it back the next time it starts.
!macro NSIS_HOOK_PREUNINSTALL
  ${If} $UpdateMode <> 1
    ExecWait '"$INSTDIR\agent-plugins.exe" remove-from-path'
  ${EndIf}
!macroend

; The uninstaller removes only the files it knows, then the folder if empty.
!macro NSIS_HOOK_POSTUNINSTALL
  Delete "$INSTDIR\agent-plugins.com"
  RMDir "$INSTDIR"
!macroend
