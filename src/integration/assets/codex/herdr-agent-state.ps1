# installed by herdr
# managed by herdr; reinstalling or updating the integration overwrites this file.
# add custom hooks beside this file instead of editing it.
# HERDR_INTEGRATION_ID=codex
# HERDR_INTEGRATION_VERSION=11

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

if ($Action -eq "mail-done" -or $Action -eq "mail-blocked") {
    $parentPaneId = $env:HERDR_PARENT_TERMINAL_ID
    if ([string]::IsNullOrWhiteSpace($parentPaneId)) { $parentPaneId = $env:HERDR_PARENT_PANE_ID }
    if ([string]::IsNullOrWhiteSpace($parentPaneId)) { exit 0 }

    if ($Action -eq "mail-done") {
        if ($payload.stop_hook_active) { exit 0 }
        $mailBody = "$($payload.last_assistant_message)"
        $mailKind = "done"
        if ([string]::IsNullOrWhiteSpace($mailBody)) {
            # A turn that emitted no assistant text (e.g. a tool-only turn)
            # would otherwise produce a zero-information envelope: empty
            # subject and body_bytes 0. Say so explicitly instead.
            $mailBody = "(no message)"
            $mailSubject = "done (no message)"
        } else {
            $mailSubject = ($mailBody -split "`n")[0]
        }
    } else {
        # PermissionRequest's payload carries tool_name/tool_input, not a
        # free-text message field; build a best-effort human summary.
        $toolName = "$($payload.tool_name)"
        $mailBody = if ([string]::IsNullOrWhiteSpace($toolName)) { "needs permission" } else { "needs permission: $toolName" }
        $mailKind = "blocked"
        $mailSubject = $mailBody
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

if ($payload.hook_event_name -and $Action -eq "session" -and $payload.hook_event_name -ne "SessionStart") { exit 0 }

$sessionId = $payload.session_id
if ([string]::IsNullOrWhiteSpace($sessionId)) { exit 0 }

$seq = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
try {
    $args = @(
        "pane",
        "report-agent-session",
        $env:HERDR_PANE_ID,
        "--source",
        "herdr:codex",
        "--agent",
        "codex",
        "--seq",
        "$seq",
        "--agent-session-id",
        "$sessionId"
    )
    if ($payload.hook_event_name -eq "SessionStart" -and $payload.source -is [string] -and -not [string]::IsNullOrWhiteSpace($payload.source)) {
        $args += @("--session-start-source", "$($payload.source)")
    }
    & herdr @args 2>$null | Out-Null
} catch {
}
