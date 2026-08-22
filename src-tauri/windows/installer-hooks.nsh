!macro NSIS_HOOK_POSTINSTALL
  ; Tauri always creates the Start menu shortcut. Make the desktop shortcut
  ; unconditional as well instead of leaving it as an installer-page choice.
  Call CreateOrUpdateDesktopShortcut
!macroend
