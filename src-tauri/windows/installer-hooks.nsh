; 随译 NSIS 安装钩子（由 tauri.conf.json 的 bundle.windows.nsis.installerHooks 引入）
;
; 作用：中文系统上把开始菜单、桌面快捷方式和“设置 → 应用”里的名称显示为「随译」，其他语言保持「Translate Anything」；
; 每次安装 / 更新都按当前系统语言重新判断，两个方向都会改。
; Tauri 自带模板总是按 productName 建快捷方式，这里在它建完之后改名；
; 卸载时把中文名的快捷方式一并删掉。
;
; 本文件必须保存为带 BOM 的 UTF-8，否则 makensis 会按系统代码页读取中文。

!include LogicLib.nsh

!define ZH_SHORTCUT_NAME "随译"

Var ZhShortcut

; 当前用户界面语言是否为中文（简繁都算）：结果写入 $ZhShortcut（1 / 0）
!macro _SuiyiDetectChinese
  System::Call 'kernel32::GetUserDefaultUILanguage() i .r0'
  IntOp $0 $0 & 0x3FF
  ${If} $0 = 0x04
    StrCpy $ZhShortcut 1
  ${Else}
    StrCpy $ZhShortcut 0
  ${EndIf}
!macroend

; 把 <目录>\<FROM>.lnk 改名为 <目录>\<TO>.lnk（仅当它指向本程序；目标已存在则覆盖）
!macro _SuiyiMoveLnk DIR FROM TO
  ${If} ${FileExists} "${DIR}\${FROM}.lnk"
    !insertmacro IsShortcutTarget "${DIR}\${FROM}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    Pop $0
    ${If} $0 = 1
      Delete "${DIR}\${TO}.lnk"
      Rename "${DIR}\${FROM}.lnk" "${DIR}\${TO}.lnk"
    ${EndIf}
  ${EndIf}
!macroend

; 按 $ZhShortcut 把 <目录> 下的快捷方式改成对应语言的名字（双向：系统语言改过后，更新时也会跟着改回）
!macro _SuiyiLocalizeLnk DIR
  ${If} $ZhShortcut = 1
    !insertmacro _SuiyiMoveLnk "${DIR}" "${PRODUCTNAME}" "${ZH_SHORTCUT_NAME}"
  ${Else}
    !insertmacro _SuiyiMoveLnk "${DIR}" "${ZH_SHORTCUT_NAME}" "${PRODUCTNAME}"
  ${EndIf}
!macroend

; 删除 <目录>\随译.lnk（仅当它指向本程序）
!macro _SuiyiDeleteLnk DIR
  ${If} ${FileExists} "${DIR}\${ZH_SHORTCUT_NAME}.lnk"
    !insertmacro IsShortcutTarget "${DIR}\${ZH_SHORTCUT_NAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    Pop $0
    ${If} $0 = 1
      !insertmacro UnpinShortcut "${DIR}\${ZH_SHORTCUT_NAME}.lnk"
      Delete "${DIR}\${ZH_SHORTCUT_NAME}.lnk"
    ${EndIf}
  ${EndIf}
!macroend

; 桌面快捷方式改名（函数里不能引用模板后面才声明的 $AppStartMenuFolder，开始菜单留给安装钩子处理）
Function SuiyiLocalizeDesktopShortcut
  Push $0
  !insertmacro _SuiyiDetectChinese
  !insertmacro _SuiyiLocalizeLnk "$DESKTOP"
  Pop $0
FunctionEnd

; 安装 / 更新完成后：开始菜单快捷方式（以及静默 / 被动安装时的桌面快捷方式）已经建好
!macro NSIS_HOOK_POSTINSTALL
  Push $0
  !insertmacro _SuiyiDetectChinese
  ; “设置 → 应用”列表里的名称（模板已写入英文名，中文系统覆盖为中文名）
  ${If} $ZhShortcut = 1
    WriteRegStr SHCTX "${UNINSTKEY}" "DisplayName" "${ZH_SHORTCUT_NAME}"
  ${EndIf}
  ${If} "$AppStartMenuFolder" != ""
    !insertmacro _SuiyiLocalizeLnk "$SMPROGRAMS\$AppStartMenuFolder"
  ${EndIf}
  !insertmacro _SuiyiLocalizeLnk "$SMPROGRAMS"
  !insertmacro _SuiyiLocalizeLnk "$DESKTOP"
  Pop $0
!macroend

; 图形安装时桌面快捷方式在“完成”页才建，所以界面关闭时再处理一次
Function .onGUIEnd
  Call SuiyiLocalizeDesktopShortcut
FunctionEnd

; 卸载（非更新）时删掉中文名快捷方式；英文名的由 Tauri 模板自己删
!macro NSIS_HOOK_PREUNINSTALL
  ${If} $UpdateMode <> 1
    Push $0
    !insertmacro MUI_STARTMENU_GETFOLDER Application $AppStartMenuFolder
    ${If} "$AppStartMenuFolder" != ""
      !insertmacro _SuiyiDeleteLnk "$SMPROGRAMS\$AppStartMenuFolder"
    ${EndIf}
    !insertmacro _SuiyiDeleteLnk "$SMPROGRAMS"
    !insertmacro _SuiyiDeleteLnk "$DESKTOP"
    Pop $0
  ${EndIf}
!macroend
