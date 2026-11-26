; Registers edits as a PDF handler the user can choose, without taking over
; their default (PRD 10): it appears in "Open with" and in
; Settings > Apps > Default apps. Tauri's own association writes the
; edits.pdf class; this adds the entries Windows uses to offer it.

!macro NSIS_HOOK_POSTINSTALL
  WriteRegStr SHCTX "Software\Classes\.pdf\OpenWithProgids" "edits.pdf" ""
  WriteRegStr SHCTX "Software\Classes\Applications\edits.exe\SupportedTypes" ".pdf" ""
  WriteRegStr SHCTX "Software\Classes\Applications\edits.exe" "FriendlyAppName" "edits"
  WriteRegStr SHCTX "Software\edits\Capabilities" "ApplicationName" "edits"
  WriteRegStr SHCTX "Software\edits\Capabilities" "ApplicationDescription" "Fast PDF reader and editor"
  WriteRegStr SHCTX "Software\edits\Capabilities\FileAssociations" ".pdf" "edits.pdf"
  WriteRegStr SHCTX "Software\RegisteredApplications" "edits" "Software\edits\Capabilities"
  ; Tell Explorer the associations changed.
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  DeleteRegValue SHCTX "Software\Classes\.pdf\OpenWithProgids" "edits.pdf"
  DeleteRegKey SHCTX "Software\Classes\Applications\edits.exe"
  DeleteRegKey SHCTX "Software\edits\Capabilities"
  DeleteRegKey /ifempty SHCTX "Software\edits"
  DeleteRegValue SHCTX "Software\RegisteredApplications" "edits"
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
!macroend
