param(
    [string]$InstallationRoot = $PSScriptRoot
)

$ErrorActionPreference = 'Stop'

function Get-CurrentRuntimeRoot {
    param([string]$Root)

    $marker = Join-Path $Root 'current_runtime_payload.txt'
    if (-not (Test-Path -LiteralPath $marker -PathType Leaf)) {
        throw "Missing runtime payload marker: $marker"
    }
    $relative = (Get-Content -LiteralPath $marker -Raw).Trim()
    if ([string]::IsNullOrWhiteSpace($relative)) {
        throw "Runtime payload marker is empty: $marker"
    }
    return [System.IO.Path]::GetFullPath((Join-Path $Root $relative)).TrimEnd('\')
}

function Get-StaleRuntimeHosts {
    param(
        [string]$Root,
        [string]$CurrentRuntimeRoot
    )

    $runtimeRoot = [System.IO.Path]::GetFullPath((Join-Path $Root 'runtime')).TrimEnd('\') + '\'
    $currentPrefix = $CurrentRuntimeRoot.TrimEnd('\') + '\'
    $protectedNames = @(
        'ApplicationFrameHost',
        'explorer',
        'SearchHost',
        'ShellExperienceHost',
        'StartMenuExperienceHost',
        'SystemSettings'
    )
    $hosts = @{}

    foreach ($process in @(Get-Process -ErrorAction SilentlyContinue)) {
        if ($protectedNames -contains $process.ProcessName) { continue }
        try {
            foreach ($module in @($process.Modules)) {
                $fileName = [string]$module.FileName
                if ([string]::IsNullOrWhiteSpace($fileName)) { continue }
                if (-not [System.IO.Path]::GetFileName($fileName).Equals('srf_tsf_tip.dll', [System.StringComparison]::OrdinalIgnoreCase)) { continue }

                $loadedDll = [System.IO.Path]::GetFullPath($fileName)
                if (-not $loadedDll.StartsWith($runtimeRoot, [System.StringComparison]::OrdinalIgnoreCase)) { continue }
                if ($loadedDll.StartsWith($currentPrefix, [System.StringComparison]::OrdinalIgnoreCase)) { continue }

                $cim = Get-CimInstance -ClassName Win32_Process -Filter ("ProcessId = {0}" -f $process.Id) -ErrorAction SilentlyContinue
                $commandLine = if ($cim) { [string]$cim.CommandLine } else { '' }
                $executablePath = if ($cim) { [string]$cim.ExecutablePath } else { '' }
                if ([string]::IsNullOrWhiteSpace($executablePath)) {
                    try { $executablePath = [string]$process.Path } catch {}
                }

                $hosts[$process.Id] = [pscustomobject]@{
                    Id = [int]$process.Id
                    ProcessName = [string]$process.ProcessName
                    ExecutablePath = $executablePath
                    CommandLine = $commandLine
                    LoadedDll = $loadedDll
                }
                break
            }
        } catch {
            # Access-denied processes are expected and cannot be safely restarted here.
        }
    }

    return @($hosts.Values | Sort-Object ProcessName, Id)
}

function Restart-SelectedHost {
    param($HostInfo)

    $process = Get-Process -Id $HostInfo.Id -ErrorAction SilentlyContinue
    if (-not $process) { return '应用已经退出。' }
    if ($process.MainWindowHandle -eq 0) { return '没有可正常关闭的窗口，已保持运行。' }

    $commandLine = [string]$HostInfo.CommandLine
    if ([string]::IsNullOrWhiteSpace($commandLine)) {
        if ([string]::IsNullOrWhiteSpace([string]$HostInfo.ExecutablePath)) {
            return '无法确定重启命令，已保持运行。'
        }
        $commandLine = ('"{0}"' -f $HostInfo.ExecutablePath)
    }

    if (-not $process.CloseMainWindow()) { return '应用拒绝关闭请求，已保持运行。' }
    if (-not $process.WaitForExit(15000)) { return '等待应用退出超时，已保持运行。' }

    try {
        $result = ([wmiclass]'Win32_Process').Create($commandLine)
        if ($result.ReturnValue -eq 0) { return '已重启。' }
        return ('重新启动失败，错误码 {0}。' -f $result.ReturnValue)
    } catch {
        return ('重新启动失败：{0}' -f $_.Exception.Message)
    }
}

$InstallationRoot = [System.IO.Path]::GetFullPath($InstallationRoot)
$currentRuntimeRoot = Get-CurrentRuntimeRoot -Root $InstallationRoot
$hosts = @(Get-StaleRuntimeHosts -Root $InstallationRoot -CurrentRuntimeRoot $currentRuntimeRoot)

Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
[System.Windows.Forms.Application]::EnableVisualStyles()

if ($hosts.Count -eq 0) {
    [void][System.Windows.Forms.MessageBox]::Show(
        '没有检测到仍在使用旧版输入法的应用。新打开的应用会直接使用新版。',
        '开心输入法',
        [System.Windows.Forms.MessageBoxButtons]::OK,
        [System.Windows.Forms.MessageBoxIcon]::Information)
    exit 0
}

$form = New-Object System.Windows.Forms.Form
$form.Text = '选择要重启的软件 - 开心输入法'
$form.StartPosition = 'CenterScreen'
$form.ClientSize = New-Object System.Drawing.Size(680, 430)
$form.MinimizeBox = $false
$form.MaximizeBox = $false
$form.FormBorderStyle = 'FixedDialog'

$intro = New-Object System.Windows.Forms.Label
$intro.Location = New-Object System.Drawing.Point(16, 14)
$intro.Size = New-Object System.Drawing.Size(648, 54)
$intro.Text = "以下应用仍在使用旧版输入法。请先保存正在编辑的内容，再勾选需要正常关闭并重启的应用。`r`n不勾选或点击取消也可以，应用下次启动时会自动使用新版。"
$form.Controls.Add($intro)

$list = New-Object System.Windows.Forms.CheckedListBox
$list.Location = New-Object System.Drawing.Point(16, 76)
$list.Size = New-Object System.Drawing.Size(648, 280)
$list.CheckOnClick = $true
$list.HorizontalScrollbar = $true
foreach ($hostInfo in $hosts) {
    $path = if ([string]::IsNullOrWhiteSpace([string]$hostInfo.ExecutablePath)) { $hostInfo.ProcessName } else { $hostInfo.ExecutablePath }
    [void]$list.Items.Add(('{0} (PID {1})  {2}' -f $hostInfo.ProcessName, $hostInfo.Id, $path), $false)
}
$form.Controls.Add($list)

$restartButton = New-Object System.Windows.Forms.Button
$restartButton.Text = '重启已勾选的软件'
$restartButton.Location = New-Object System.Drawing.Point(420, 378)
$restartButton.Size = New-Object System.Drawing.Size(150, 34)
$restartButton.Add_Click({ $form.DialogResult = [System.Windows.Forms.DialogResult]::OK; $form.Close() })
$form.Controls.Add($restartButton)

$cancelButton = New-Object System.Windows.Forms.Button
$cancelButton.Text = '暂不重启'
$cancelButton.Location = New-Object System.Drawing.Point(580, 378)
$cancelButton.Size = New-Object System.Drawing.Size(84, 34)
$cancelButton.Add_Click({ $form.DialogResult = [System.Windows.Forms.DialogResult]::Cancel; $form.Close() })
$form.Controls.Add($cancelButton)
$form.AcceptButton = $restartButton
$form.CancelButton = $cancelButton

if ($form.ShowDialog() -ne [System.Windows.Forms.DialogResult]::OK) { exit 0 }

$selected = @($list.CheckedIndices | ForEach-Object { $hosts[[int]$_] })
if ($selected.Count -eq 0) { exit 0 }

$results = foreach ($hostInfo in $selected) {
    '{0} (PID {1})：{2}' -f $hostInfo.ProcessName, $hostInfo.Id, (Restart-SelectedHost -HostInfo $hostInfo)
}
[void][System.Windows.Forms.MessageBox]::Show(
    ($results -join "`r`n"),
    '开心输入法 - 重启结果',
    [System.Windows.Forms.MessageBoxButtons]::OK,
    [System.Windows.Forms.MessageBoxIcon]::Information)

