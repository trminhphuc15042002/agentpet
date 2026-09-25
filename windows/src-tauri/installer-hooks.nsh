; Rust's GNU Windows target dynamically loads WebView2Loader.dll from beside
; agentpet.exe. Tauri resources live under $INSTDIR\resources, so copy this
; runtime dependency beside the executable after the standard install finishes.
!macro NSIS_HOOK_POSTINSTALL
  CopyFiles /SILENT "$INSTDIR\resources\WebView2Loader.dll" "$INSTDIR\WebView2Loader.dll"
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  Delete "$INSTDIR\WebView2Loader.dll"
!macroend
