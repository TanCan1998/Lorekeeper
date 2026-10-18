; 仅扩展当前用户安装；不删除共享 WebView2、其他应用或最近项目列表。
!macro NSIS_HOOK_PREUNINSTALL
  ; 接管模板的“删除应用数据”勾选框，禁止模板随后按 AppData 路径递归删除。
  StrCpy $R8 $DeleteAppDataCheckboxState
  StrCpy $DeleteAppDataCheckboxState 0
  ; 安装器维护/升级调用会携带 _?=，也必须无条件保留项目。
  ClearErrors
  ${GetOptions} $CMDLINE "_?=" $R9
  ${If} ${Errors}
  ${AndIf} $R8 = 1
  ${AndIf} $UpdateMode <> 1
  ${AndIf} $PassiveMode <> 1
  ${AndIfNot} ${Silent}
    !insertmacro CheckIfAppIsRunning "$INSTDIR\${MAINBINARYNAME}.exe" "${PRODUCTNAME}"
    IfFileExists "$INSTDIR\${MAINBINARYNAME}.exe" 0 lorekeeper_keep_projects
    ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" --cleanup-project' $0
    lorekeeper_keep_projects:
  ${EndIf}
  ClearErrors
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
    ; Tauri 模板负责卸载项与 AppUserModelId；本应用安装位置/语言键
    ; 无论是否保留项目都清理。共享厂商键仅在空时移除。
    DeleteRegKey SHCTX "${MANUPRODUCTKEY}"
    DeleteRegKey /ifempty SHCTX "${MANUKEY}"
    DeleteRegValue HKCU "${MANUPRODUCTKEY}" "Installer Language"
    DeleteRegKey /ifempty HKCU "${MANUPRODUCTKEY}"
    DeleteRegKey /ifempty HKCU "${MANUKEY}"
  ${EndIf}
!macroend
