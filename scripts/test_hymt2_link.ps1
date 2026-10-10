param([string]$OutputPath = '', [string]$ImeClient = '')
$ErrorActionPreference = 'Stop'
$Utf8 = [System.Text.UTF8Encoding]::new($false)
$OutputEncoding = $Utf8
function Assert-True($Condition, [string]$Message) { if (-not $Condition) { throw $Message } }
function Send-Request($Request) {
    $client = [System.IO.Pipes.NamedPipeClientStream]::new('.', 'HyMT2.IME.Request', [System.IO.Pipes.PipeDirection]::InOut, [System.IO.Pipes.PipeOptions]::Asynchronous)
    try {
        $client.Connect(5000)
        $bytes = $Utf8.GetBytes(($Request | ConvertTo-Json -Depth 8 -Compress) + "`n")
        $client.Write($bytes, 0, $bytes.Length)
        $reader = [System.IO.StreamReader]::new($client, $Utf8, $false, 4096, $true)
        try {
            $reading = $reader.ReadLineAsync()
            if (-not $reading.Wait(5000)) { throw 'Request acknowledgement exceeded 5 seconds' }
            return ($reading.Result | ConvertFrom-Json)
        } finally { $reader.Dispose() }
    } finally { $client.Dispose() }
}
function New-Callback([string]$Name) {
    $pipe = [System.IO.Pipes.NamedPipeServerStream]::new($Name, [System.IO.Pipes.PipeDirection]::In, 1, [System.IO.Pipes.PipeTransmissionMode]::Byte, [System.IO.Pipes.PipeOptions]::Asynchronous, 65536, 65536)
    return @{ Pipe=$pipe; Waiting=$pipe.WaitForConnectionAsync() }
}
function Receive-Event($Server, [int]$Timeout = 60000) {
    $waiting = $Server.Waiting
    if (-not $waiting.Wait($Timeout)) { throw 'Callback connection timeout' }
    $reader = [System.IO.StreamReader]::new($Server.Pipe, $Utf8, $false, 4096, $true)
    try {
        $reading = $reader.ReadLineAsync()
        if (-not $reading.Wait(5000)) { throw 'Callback read timeout' }
        return ($reading.Result | ConvertFrom-Json)
    } finally { $reader.Dispose() }
}
function Run-Translation([string]$Text, [bool]$Cancel = $false) {
    $id = [Guid]::NewGuid().ToString('N')
    $name = "Kaixin.Translate.Result.$id"
    $server = New-Callback $name
    $request = @{ protocol_version=2; request_id=$id; action='translate'; text=$Text; source='auto'; target='auto-opposite'; presentation='background'; delivery='return'; reply_pipe=$name; target_hwnd=123; target_process_id=456; focus_generation=789; replace_selection=$true }
    $events = @()
    try {
        $timer = [Diagnostics.Stopwatch]::StartNew()
        if ($ImeClient) {
            $request.origin = 'kaixin-ime-integration-test'
            $request.result_action = 'show'
            $request.interactive = $false
            $senderOutput = ($request | ConvertTo-Json -Depth 8 -Compress) | & $ImeClient send
            Assert-True ($LASTEXITCODE -eq 0) 'Actual IME client rejected translation'
            $accepted = @{ok=$true}
        } else { $accepted = Send-Request $request }
        $ackMs = $timer.ElapsedMilliseconds
        Assert-True $accepted.ok 'Translation was rejected'
        Assert-True ($ackMs -lt 2500) 'Acknowledgement waited for model inference'
        if ($Cancel) {
            $cancelled = Send-Request @{protocol_version=2; request_id="cancel-$id"; action='cancel'; cancel_request_id=$id}
            Assert-True ($cancelled.ok -and $cancelled.cancelled) 'Cancel was not accepted'
        }
        for ($count = 0; $count -lt 30; $count++) {
            $event = Receive-Event $server
            Assert-True ($event.request_id -eq $id) 'Wrong request identity in callback'
            Assert-True ($event.target_hwnd -eq 123 -and $event.target_process_id -eq 456 -and $event.focus_generation -eq 789 -and $event.replace_selection) 'Callback lost target metadata'
            $events += $event.event
            $server.Pipe.Dispose()
            if ($event.event -in @('completed','failed','cancelled')) { break }
            $server = New-Callback $name
        }
        if ($Cancel) { Assert-True ($event.event -eq 'cancelled') 'Cancelled request produced a different terminal event' }
        else {
            Assert-True ($event.event -eq 'completed') "Translation failed: $($event.message)"
            Assert-True (-not [string]::IsNullOrWhiteSpace($event.text)) 'Translation result is empty'
            if ($Text -match '[\u4e00-\u9fff]') { Assert-True ($event.text -match '[A-Za-z]') 'Chinese source did not produce English text' }
            else { Assert-True ($event.text -match '[\u4e00-\u9fff]') 'English source did not produce Chinese text' }
        }
        $duplicate = Send-Request $request
        Assert-True ($duplicate.ok -and $duplicate.duplicate) 'Completed request was executed again'
        return @{ cancelled=$Cancel; acknowledgement_ms=$ackMs; events=$events; result=$event.text }
    } finally { $server.Pipe.Dispose() }
}
$capabilities = Send-Request @{protocol_version=2; request_id=[Guid]::NewGuid().ToString('N'); action='capabilities'}
Assert-True ($capabilities.ok -and $capabilities.provider -eq 'hy-mt2') 'Wrong translation provider'
Assert-True ($capabilities.capabilities.actions -contains 'cancel') 'Missing cancel capability'
Assert-True ($capabilities.capabilities.callback_events -contains 'completed') 'Missing completion capability'
$badPipe = Send-Request @{protocol_version=2; request_id='invalid-callback'; action='translate'; text='hello'; target='zh'; reply_pipe='Other.Pipe'; presentation='background'; delivery='return'}
Assert-True (-not $badPipe.ok) 'Unsafe callback was accepted'
$badLanguage = Send-Request @{protocol_version=2; request_id='invalid-language'; action='translate'; text='hello'; target='unknown'}
Assert-True (-not $badLanguage.ok) 'Unknown language was accepted'
$badVersion = Send-Request @{protocol_version=1; request_id='invalid-version'; action='ping'}
Assert-True (-not $badVersion.ok) 'Unsupported protocol version was accepted'
$results = @(
    (Run-Translation 'The local translator keeps all text on this computer.'),
    (Run-Translation '你好，这是一条来自开心输入法的本地翻译请求。'),
    (Run-Translation (('This is a long translation request that should be cancelled. ' * 300)) $true)
)
$report = @{ passed=$true; provider=$capabilities.provider; model_ready=$capabilities.model_ready; engine_ready=$capabilities.engine_ready; translations=$results; rejected_invalid_callback=$true; rejected_unknown_language=$true; rejected_wrong_version=$true }
if ($OutputPath) { [IO.File]::WriteAllText($OutputPath, ($report | ConvertTo-Json -Depth 10), $Utf8) }
$report | ConvertTo-Json -Depth 10
