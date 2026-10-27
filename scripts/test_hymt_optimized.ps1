param([string]$ReportPath = (Join-Path $PSScriptRoot '..\dist\hymt-optimized-integration.json'))
$ErrorActionPreference = 'Stop'
$Utf8 = [Text.UTF8Encoding]::new($false)
function Request($Value) {
    $pipe = [IO.Pipes.NamedPipeClientStream]::new('.', 'HyMT2.IME.Request', [IO.Pipes.PipeDirection]::InOut, [IO.Pipes.PipeOptions]::Asynchronous)
    try {
        $pipe.Connect(3000)
        $bytes = $Utf8.GetBytes(($Value | ConvertTo-Json -Depth 12 -Compress) + "`n")
        $pipe.Write($bytes, 0, $bytes.Length)
        $reader = [IO.StreamReader]::new($pipe, $Utf8, $false, 4096, $true)
        try { $read = $reader.ReadLineAsync(); if (-not $read.Wait(5000)) { throw 'ACK timeout' }; return $read.Result | ConvertFrom-Json } finally { $reader.Dispose() }
    } finally { $pipe.Dispose() }
}
function Assert($Value, $Message) { if (-not $Value) { throw $Message } }
function Query([string]$Id, [int]$Chunk = 0) {
    return Request @{protocol_version=2; request_id=[guid]::NewGuid().ToString('N'); action='status'; query_request_id=$Id; chunk_index=$Chunk}
}
function Wait-Result([string]$Id) {
    $clock = [Diagnostics.Stopwatch]::StartNew()
    do {
        $value = Query $Id
        Assert $value.ok 'Status rejected'
        if ($value.result.event -in @('completed','failed','cancelled')) { return $value }
        Start-Sleep -Milliseconds 100
    } while ($clock.Elapsed.TotalSeconds -lt 120)
    throw 'Translation timeout'
}
function New-Translation([string]$Text, [string]$Session, [int]$Generation = 1) {
    $id = [guid]::NewGuid().ToString('N')
    return @{protocol_version=2; request_id=$id; action='translate'; text=$Text; source='auto'; target='auto-opposite';
        presentation='background'; delivery='return'; reply_pipe="Kaixin.Translate.Result.$id"; callback_ack=$true;
        origin='kaixin-optimized-test'; session=$Session; generation=$Generation; target_hwnd=123; target_process_id=456; focus_generation=789; replace_selection=$true}
}
$cap = Request @{protocol_version=2; request_id='optimized-capabilities'; action='capabilities'}
Assert ($cap.ok -and $cap.provider -eq 'hy-mt2') 'Wrong provider'
Assert ($cap.capabilities.actions -contains 'status' -and $cap.capabilities.terminal_ack) 'Reliability capabilities absent'
$timer = [Diagnostics.Stopwatch]::StartNew()
$first = New-Translation 'Hello, the local translation is ready.' 'english'
$ack = Request $first
$ackMs = $timer.ElapsedMilliseconds
Assert ($ack.ok -and $ackMs -lt 2500) 'ACK waited for translation'
# Intentionally no callback listener: complete via status, proving recovery.
$done = Wait-Result $first.request_id
Assert ($done.result.event -eq 'completed' -and $done.result.text -match '[\u4e00-\u9fff]') "English translation failed: $($done.result.message)"
Assert ($done.result.target_hwnd -eq 123 -and $done.result.focus_generation -eq 789) 'Status lost target metadata'
$dup = Request $first
Assert ($dup.ok -and $dup.duplicate) 'Duplicate not deduplicated'
$changed = $first.Clone(); $changed.text = 'A different payload.'
Assert (-not (Request $changed).ok) 'Same ID accepted a different payload'
$acknowledged = Request @{protocol_version=2; request_id='optimized-ack'; action='ack'; query_request_id=$first.request_id}
Assert ($acknowledged.ok -and $acknowledged.state -eq 'delivered') 'Terminal ACK did not persist'
$chinese = New-Translation '你好，本地翻译已经准备就绪。' 'chinese'
Assert (Request $chinese).ok 'Chinese request rejected'
$zhDone = Wait-Result $chinese.request_id
Assert ($zhDone.result.event -eq 'completed' -and $zhDone.result.text -match '[A-Za-z]') 'Chinese translation failed'
$old = New-Translation ('Translate this paragraph carefully. ' * 500) 'supersede' 1
Assert (Request $old).ok 'Old session rejected'
$new = New-Translation 'Good morning.' 'supersede' 2
Assert (Request $new).ok 'Replacement rejected'
Assert ((Wait-Result $old.request_id).result.event -eq 'cancelled') 'Superseded request completed'
Assert ((Wait-Result $new.request_id).result.event -eq 'completed') 'Replacement did not complete'
$stale = New-Translation 'Stale request.' 'supersede' 1
# Complete task generations must still prevent stale requests from restarting.
Assert (-not (Request $stale).ok) 'A stale generation restarted after completion'
$cancel = New-Translation ('Cancel this queued text. ' * 500) 'cancel'
Assert (Request $cancel).ok 'Cancelable task rejected'
$cancelAck = Request @{protocol_version=2; request_id='optimized-cancel'; action='cancel'; cancel_request_id=$cancel.request_id}
Assert ($cancelAck.ok -and $cancelAck.cancelled) 'Cancellation not accepted'
Assert ((Wait-Result $cancel.request_id).result.event -eq 'cancelled') 'Cancellation did not finish'
$cached = New-Translation $first.text 'cached'
$timer.Restart(); Assert (Request $cached).ok 'Cached task rejected'; $cacheDone = Wait-Result $cached.request_id
Assert ($cacheDone.result.text -eq $done.result.text) 'Cache result differs'
$cacheMs = $timer.ElapsedMilliseconds
$numeric = New-Translation '订单 {order_id} 金额 123.45 元，比例 18%，链接 https://example.com/path?q=1' 'numbers'
$numeric.target = 'en'
Assert (Request $numeric).ok 'Numeric request rejected'
$numericDone = Wait-Result $numeric.request_id
Assert ($numericDone.result.event -eq 'completed') "Numeric translation failed: $($numericDone.result.message)"
Assert ($numericDone.result.text.Contains('{order_id}') -and $numericDone.result.text.Contains('123.45') -and $numericDone.result.text.Contains('18%') -and $numericDone.result.text.Contains('https://example.com/path?q=1')) 'Numbers or placeholders changed'
$report = @{passed=$true; provider=$cap.provider; acknowledgement_ms=$ackMs; cached_roundtrip_ms=$cacheMs;
    callback_unavailable_recovered=$true; duplicate_payload_checked=$true; terminal_ack=$true; session_supersession=$true; cancelled=$true;
    english_to_chinese=$done.result.text; chinese_to_english=$zhDone.result.text; numbers_urls_placeholders_preserved=$true; protected_translation=$numericDone.result.text}
[IO.File]::WriteAllText($ReportPath, ($report | ConvertTo-Json -Depth 8), $Utf8)
$report | ConvertTo-Json -Depth 8
