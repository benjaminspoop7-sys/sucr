; The add-on half of install and removal. Both verbs run the installed exe
; before any window, database or tray exists, print their answer and exit, so
; the installer only waits for the exit code. A failure here leaves the app
; installed and usable; the Settings card can run the same step again.
!macro NSIS_HOOK_POSTINSTALL
  DetailPrint "Registering the Sucr Capture relay"
  ExecWait '"$INSTDIR\sucr.exe" --install-addon'
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  DetailPrint "Removing the Sucr Capture relay"
  ExecWait '"$INSTDIR\sucr.exe" --remove-addon'
!macroend
