; Installer hooks for Aurora (used by tauri.conf.json > bundle.windows.nsis.installerHooks).
; The Start Menu shortcut is created by the default Tauri installer.
; This hook adds the desktop shortcut and removes it when the app is uninstalled.

!macro NSIS_HOOK_POSTINSTALL
  CreateShortCut "$DESKTOP\Aurora.lnk" "$INSTDIR\Aurora.exe" "" "$INSTDIR\Aurora.exe" 0
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  Delete "$DESKTOP\Aurora.lnk"
!macroend
