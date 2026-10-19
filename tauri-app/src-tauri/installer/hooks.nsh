; 仅扩展当前用户安装；不删除共享 WebView2、其他应用或最近项目列表。

; 安装记录：身份启动器据此在用户所选目录中定位真实主程序。
; 只追加本应用键下的命名值，不改模板默认值、不移动文件、不建链接。
!macro NSIS_HOOK_POSTINSTALL
  WriteRegDWORD SHCTX "${MANUPRODUCTKEY}" "IdentityRecordSchema" 1
  WriteRegStr SHCTX "${MANUPRODUCTKEY}" "IdentityPackageName" "com.tancan1998.lorekeeper"
  WriteRegStr SHCTX "${MANUPRODUCTKEY}" "InstallScope" "user"
  WriteRegStr SHCTX "${MANUPRODUCTKEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr SHCTX "${MANUPRODUCTKEY}" "MainExecutable" "$INSTDIR\${MAINBINARYNAME}.exe"

  ; 注册未签名身份小包（当前用户）。注册只在安装器显式执行；失败不阻断安装，
  ; 但必须告知归组不可用，不能静默留下“看起来已修复”的状态。
  IfFileExists "$INSTDIR\identity\LoreKeeper.msix" 0 lorekeeper_identity_absent
    StrCpy $R7 '$\"$INSTDIR\identity\LoreKeeper.msix$\"'
    nsExec::ExecToStack '"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "Add-AppxPackage -Path $R7 -AllowUnsigned -ErrorAction Stop; exit $LASTEXITCODE"'
    Pop $R9
    Pop $R8
    StrCmp $R9 "0" 0 lorekeeper_identity_failed
      ; 未签名包不出现在应用列表，入口必须直接运行包内启动器；登记其真实路径。
      nsExec::ExecToStack '"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "$p=Get-AppxPackage -Name com.tancan1998.lorekeeper; if($p){[Console]::Out.Write($p.InstallLocation)}"'
      Pop $R8
      Pop $R6
      StrCmp $R6 "" lorekeeper_identity_failed
      WriteRegStr SHCTX "${MANUPRODUCTKEY}" "IdentityLauncherPath" "$R6\lorekeeper-identity-launcher.exe"
      Goto lorekeeper_identity_done
    lorekeeper_identity_failed:
      ${IfNot} ${Silent}
        MessageBox MB_ICONEXCLAMATION|MB_OK "LoreKeeper 身份组件注册失败（$R9）。应用仍可使用，但任务管理器不会把 WebView2 进程归入同一应用组。"
      ${EndIf}
    Goto lorekeeper_identity_done
  lorekeeper_identity_absent:
    ${IfNot} ${Silent}
      MessageBox MB_ICONEXCLAMATION|MB_OK "安装包缺少身份组件，任务管理器归组不可用。请重新下载完整安装包。"
    ${EndIf}
  lorekeeper_identity_done:

  ; 入口改为激活器：无包身份时只激活身份包，不会无身份启动主程序。
  IfFileExists "$INSTDIR\lorekeeper-activate.exe" 0 lorekeeper_activate_absent
    IfFileExists "$SMPROGRAMS\${PRODUCTNAME}.lnk" 0 +2
      CreateShortcut "$SMPROGRAMS\${PRODUCTNAME}.lnk" "$INSTDIR\lorekeeper-activate.exe" "" "$INSTDIR\lorekeeper-activate.exe" 0
    IfFileExists "$DESKTOP\${PRODUCTNAME}.lnk" 0 +2
      CreateShortcut "$DESKTOP\${PRODUCTNAME}.lnk" "$INSTDIR\lorekeeper-activate.exe" "" "$INSTDIR\lorekeeper-activate.exe" 0
  lorekeeper_activate_absent:
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; 本应用把快捷方式改指了激活器，模板只认主程序目标，这里先删除自己的快捷方式。
  ${If} $UpdateMode <> 1
    Delete "$SMPROGRAMS\${PRODUCTNAME}.lnk"
    Delete "$DESKTOP\${PRODUCTNAME}.lnk"
    ; 只注销当前用户的身份包；不影响其他用户或系统置备。
    nsExec::ExecToStack '"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "Get-AppxPackage -Name com.tancan1998.lorekeeper | Remove-AppxPackage -ErrorAction SilentlyContinue; exit 0"'
    Pop $R9
    Pop $R8
  ${EndIf}
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
