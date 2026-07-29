# installed by herdr
# managed by herdr; reinstalling or updating the integration overwrites this file.
# add custom hooks beside this file instead of editing it.
# HERDR_INTEGRATION_ID=claude
# HERDR_INTEGRATION_VERSION=12

param([string]$Action = "")

if ($Action -notin @("session", "mail-done", "mail-blocked")) { exit 0 }
if ($env:HERDR_ENV -ne "1") { exit 0 }
if ([string]::IsNullOrWhiteSpace($env:HERDR_PANE_ID)) { exit 0 }

$inputText = [Console]::In.ReadToEnd()
try {
    $payload = if ([string]::IsNullOrWhiteSpace($inputText)) { $null } else { $inputText | ConvertFrom-Json }
} catch {
    exit 0
}

if (-not [string]::IsNullOrWhiteSpace($payload.agent_id)) { exit 0 }
if ($payload.hook_event_name -eq "SubagentStop") { exit 0 }

if ($Action -eq "mail-done" -or $Action -eq "mail-blocked") {
    $parentPaneId = $env:HERDR_PARENT_TERMINAL_ID
    if ([string]::IsNullOrWhiteSpace($parentPaneId)) { $parentPaneId = $env:HERDR_PARENT_PANE_ID }
    if ([string]::IsNullOrWhiteSpace($parentPaneId)) { exit 0 }

    if ($Action -eq "mail-done") {
        if ($payload.stop_hook_active) { exit 0 }
        $mailBody = "$($payload.last_assistant_message)"
        $mailKind = "done"
        $mailSubject = ($mailBody -split "`n")[0]
    } else {
        $mailBody = "$($payload.message)"
        $mailKind = "blocked"
        $mailSubject = $mailBody
        if ([string]::IsNullOrWhiteSpace($mailSubject)) { $mailSubject = "needs attention" }
    }
    if ($mailSubject.Length -gt 120) { $mailSubject = $mailSubject.Substring(0, 120) }

    try {
        $mailArgs = @(
            "mail",
            "send",
            "$parentPaneId",
            "--kind",
            "$mailKind",
            "--subject",
            "$mailSubject",
            "--body-stdin"
        )
        $mailBody | & herdr @mailArgs 2>$null | Out-Null
    } catch {
    }
    exit 0
}

$sessionId = $payload.session_id
if ([string]::IsNullOrWhiteSpace($sessionId)) { exit 0 }

$seq = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
try {
    $args = @(
        "pane",
        "report-agent-session",
        $env:HERDR_PANE_ID,
        "--source",
        "herdr:claude",
        "--agent",
        "claude",
        "--seq",
        "$seq",
        "--agent-session-id",
        "$sessionId"
    )
    if ($payload.transcript_path -is [string] -and -not [string]::IsNullOrWhiteSpace($payload.transcript_path)) {
        $args += @("--agent-session-path", "$($payload.transcript_path)")
    }
    if ($payload.hook_event_name -eq "SessionStart" -and $payload.source -is [string] -and -not [string]::IsNullOrWhiteSpace($payload.source)) {
        $args += @("--session-start-source", "$($payload.source)")
    }
    & herdr @args 2>$null | Out-Null
} catch {
}

if ($Action -eq "session") {
@'
__HERDR_SESSION_DOCTRINE__
'@
}
