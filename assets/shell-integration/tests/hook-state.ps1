# Executed after installing the real hook, in an isolated -NoProfile test process.
# Only input acquisition is stubbed; boundary and prompt handling are real.
function Assert-UnknownFinish([string]$text) {
    $marker = "$([char]27)]133;D$([char]7)"
    if ([regex]::Matches($text, [regex]::Escape($marker)).Count -ne 1 -or $text.Contains(']133;D;')) {
        throw 'Expected exactly one code-free D, never a numeric exit code'
    }
}
$global:__KegonState.OriginalReadLine = { 'nonexistent_kegon_hook_probe_784392' }
$global:__KegonState.OriginalPrompt = {
    $global:KegonProbeSuccess = $?
    $global:KegonProbeExitCode = $global:LASTEXITCODE
    'probe> '
}
$global:__KegonState.HasReadLine = $true
$global:LASTEXITCODE = 42
$Error.Clear()
$line = PSConsoleHostReadLine
$hookErrorCount = $Error.Count
if ($hookErrorCount -ne 0) { throw "Input hook added $hookErrorCount errors" }
if ($global:LASTEXITCODE -ne 42) { throw 'Input hook changed LASTEXITCODE' }
if ($line -ne 'nonexistent_kegon_hook_probe_784392') { throw 'Input hook changed the input' }

# Check preservation, not just an empty history. Do not clear after invoking the hook.
Write-Error 'existing user error' -ErrorAction SilentlyContinue
$existingError = $Error[0]
$existingCount = $Error.Count
$line = PSConsoleHostReadLine
$hookErrorCount = $Error.Count
if ($hookErrorCount -ne $existingCount -or -not [object]::ReferenceEquals($existingError, $Error[0])) {
    throw 'Input hook changed existing error history'
}

# The original prompt must observe the user's status without a synthetic error record.
Write-Error 'user failure without history' -ErrorAction Ignore
$rendered = prompt
Assert-UnknownFinish $rendered
if ($global:KegonProbeSuccess -ne $false -or $global:KegonProbeExitCode -ne 42) {
    throw 'Original prompt did not observe the failing status'
}
if ($Error.Count -ne $existingCount -or -not [object]::ReferenceEquals($existingError, $Error[0])) {
    throw 'Prompt changed existing error history'
}
$global:__KegonState.Executing = $true
Write-Output 'success' | Out-Null
$rendered = prompt
Assert-UnknownFinish $rendered
if ($global:KegonProbeSuccess -ne $true -or $global:KegonProbeExitCode -ne 42) {
    throw 'Original prompt did not observe the successful status'
}
if ($Error.Count -ne $existingCount) { throw 'Successful prompt added an error' }

# Rendering the prompt again without another input must not finish another command.
$rendered = prompt
if ($rendered.Contains(']133;D')) { throw 'Repeated prompt emitted an orphaned D' }
