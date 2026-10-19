; C1 Studio: which kinds of photo source the app offers.
;
; The "Photo sources" page (added to installer.nsi after the directory page) writes the
; choice to %APPDATA%\c1studio\providers.json, which c1sources::Offered reads. Without the
; file the app offers every kind. On a reinstall the page starts from the file there.
;
; Silent and passive installs skip the page: /PROVIDERS=capture-one,folders sets the list,
; and without it a file already there is kept.
;
; The ids must match the providers' (c1sources checks every one is in this file).

!include nsDialogs.nsh
!include LogicLib.nsh

!define /ifndef C1_OFFERED_DIR "$APPDATA\c1studio"
!define C1_OFFERED "${C1_OFFERED_DIR}\providers.json"

Var C1OfferLoaded
Var C1OfferChosen
Var C1OfferCaptureOne
Var C1OfferFolders
Var C1OfferCaptureOneBox
Var C1OfferFoldersBox

; Each kind on, unless a file from before names some.
Function C1OfferLoad
  ${If} $C1OfferLoaded == 1
    Return
  ${EndIf}
  StrCpy $C1OfferLoaded 1
  StrCpy $C1OfferCaptureOne 1
  StrCpy $C1OfferFolders 1
  IfFileExists "${C1_OFFERED}" 0 done
  StrCpy $R0 ""
  ClearErrors
  FileOpen $R1 "${C1_OFFERED}" r
  IfErrors done
  read:
    FileRead $R1 $R2
    IfErrors closed
    StrCpy $R0 "$R0$R2"
    Goto read
  closed:
  FileClose $R1
  ${StrLoc} $R3 $R0 '"capture-one"' ">"
  ${StrLoc} $R4 $R0 '"folders"' ">"
  ; A list naming neither offers both, as the app reads it.
  ${If} $R3 != ""
  ${OrIf} $R4 != ""
    ${If} $R3 == ""
      StrCpy $C1OfferCaptureOne 0
    ${EndIf}
    ${If} $R4 == ""
      StrCpy $C1OfferFolders 0
    ${EndIf}
  ${EndIf}
  done:
FunctionEnd

Function C1OfferPage
  ; Passive installs show no pages.
  ClearErrors
  ${GetOptions} $CMDLINE "/P" $R0
  ${IfNot} ${Errors}
    Abort
  ${EndIf}
  Call C1OfferLoad
  !insertmacro MUI_HEADER_TEXT "Photo sources" "Choose what C1 Studio opens. You can change it later in the Source menu."
  nsDialogs::Create 1018
  Pop $R0
  ${If} $(^RTL) = 1
    nsDialogs::SetRTL $(^RTL)
  ${EndIf}

  ${NSD_CreateCheckbox} 0 10u 100% 12u "&Capture One catalogs"
  Pop $C1OfferCaptureOneBox
  ${NSD_CreateLabel} 12u 24u -12u 20u "Browse and edit a Capture One catalog (.cocatalogdb)."
  Pop $R0
  ${NSD_CreateCheckbox} 0 52u 100% 12u "&Photo folders"
  Pop $C1OfferFoldersBox
  ${NSD_CreateLabel} 12u 66u -12u 20u "Browse and edit plain folders of JPEG, PNG, TIFF and raw files."
  Pop $R0

  ${If} $C1OfferCaptureOne == 1
    ${NSD_Check} $C1OfferCaptureOneBox
  ${EndIf}
  ${If} $C1OfferFolders == 1
    ${NSD_Check} $C1OfferFoldersBox
  ${EndIf}
  nsDialogs::Show
FunctionEnd

Function C1OfferLeave
  ${NSD_GetState} $C1OfferCaptureOneBox $R0
  ${NSD_GetState} $C1OfferFoldersBox $R1
  ${If} $R0 <> ${BST_CHECKED}
  ${AndIf} $R1 <> ${BST_CHECKED}
    MessageBox MB_ICONEXCLAMATION|MB_OK "Choose at least one kind of photo source."
    Abort
  ${EndIf}
  StrCpy $C1OfferCaptureOne 0
  StrCpy $C1OfferFolders 0
  ${If} $R0 == ${BST_CHECKED}
    StrCpy $C1OfferCaptureOne 1
  ${EndIf}
  ${If} $R1 == ${BST_CHECKED}
    StrCpy $C1OfferFolders 1
  ${EndIf}
  StrCpy $C1OfferChosen 1
FunctionEnd

; /PROVIDERS=capture-one,folders, when given and naming at least one.
Function C1OfferFromCommandLine
  ClearErrors
  ${GetOptions} $CMDLINE "/PROVIDERS=" $R0
  ${If} ${Errors}
    Return
  ${EndIf}
  ${StrLoc} $R1 ",$R0," ",capture-one," ">"
  ${StrLoc} $R2 ",$R0," ",folders," ">"
  ${If} $R1 == ""
  ${AndIf} $R2 == ""
    DetailPrint "/PROVIDERS=$R0 names no kind of photo source: left as it was"
    Return
  ${EndIf}
  StrCpy $C1OfferCaptureOne 0
  StrCpy $C1OfferFolders 0
  ${If} $R1 != ""
    StrCpy $C1OfferCaptureOne 1
  ${EndIf}
  ${If} $R2 != ""
    StrCpy $C1OfferFolders 1
  ${EndIf}
  StrCpy $C1OfferChosen 1
FunctionEnd

Function C1OfferWrite
  Call C1OfferFromCommandLine
  ${If} $C1OfferChosen <> 1
    Return
  ${EndIf}
  StrCpy $R0 ""
  ${If} $C1OfferCaptureOne == 1
    StrCpy $R0 '"capture-one"'
  ${EndIf}
  ${If} $C1OfferFolders == 1
    ${If} $R0 != ""
      StrCpy $R0 "$R0, "
    ${EndIf}
    StrCpy $R0 '$R0"folders"'
  ${EndIf}
  CreateDirectory "${C1_OFFERED_DIR}"
  ClearErrors
  FileOpen $R1 "${C1_OFFERED}" w
  ${If} ${Errors}
    DetailPrint "Could not write ${C1_OFFERED}"
    Return
  ${EndIf}
  FileWrite $R1 '{ "providers": [$R0] }$\r$\n'
  FileClose $R1
  DetailPrint "Photo sources offered: $R0"
FunctionEnd

!macro NSIS_HOOK_POSTINSTALL
  Call C1OfferWrite
!macroend
