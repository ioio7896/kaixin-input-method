[Setup]
AppPublisherURL=https://github.com/ioio7896/kaixin-input-method
AppSupportURL=https://github.com/ioio7896/kaixin-input-method
LicenseFile=安装与使用条款.txt

[Messages]
WizardLicense=开心输入法安装与使用条款
ConfirmUninstall=卸载 %1 将清除当前 Windows 用户在本机的输入法配置、学习词库、剪贴板历史、OCR 历史、缓存和日志，并恢复微软拼音。%n%n请先备份需要保留的数据。保存在上述数据目录以外的截图和导出文件会保留在原位置。%n%n确定继续卸载吗？

[Files]
Source: "安装与使用条款.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "隐私说明.txt"; DestDir: "{app}"; Flags: ignoreversion
