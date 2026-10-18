# Kegon Shell Integration for PowerShell
# Supports PowerShell 7+ (pwsh) and Windows PowerShell 5.1
# Conforms to FinalTerm / OSC 133 semantic prompt specification and OSC 7 CWD reporting

try {
    # 1. Guard against multiple installation in the same session
    if ($null -ne $global:__KegonState) {
        return
    }

    # 2. Disable in restricted language modes
    if ($ExecutionContext.SessionState.LanguageMode -ne 'FullLanguage') {
        return
    }

    # 3. Require an existing prompt function to wrap
    if (-not (Get-Command -Name prompt -CommandType Function,Filter -ErrorAction Ignore)) {
        return
    }

    $global:__KegonState = @{
        OriginalPrompt = $function:prompt
        OriginalReadLine = $null
        HasReadLine = $false
        Executing = $false
        HostName = '@HOST@'
        Seq = 0
    }

    # 4. Attempt to hook PSConsoleHostReadLine for command execution start (OSC 133;C)
    # Must be in isolated try/catch because PSReadLine can throw a terminating error on unsupported hosts
    try {
        if (-not (Get-Module -Name PSReadLine)) {
            Import-Module PSReadLine -ErrorAction Stop
        }
    } catch { }

    try {
        if (Get-Command -Name PSConsoleHostReadLine -ErrorAction Ignore) {
            $global:__KegonState.OriginalReadLine = $function:PSConsoleHostReadLine
            $global:__KegonState.HasReadLine = $true

            function global:PSConsoleHostReadLine {
                $line = & $global:__KegonState.OriginalReadLine
                if ($null -ne $line -and $line.Trim().Length -gt 0) {
                    $global:__KegonState.Executing = $true
                    $global:__KegonState.Seq++
                    if ($env:KEGON_SESSION_ID) {
                        $sid = $env:KEGON_SESSION_ID
                        $seq = $global:__KegonState.Seq
                        # Scheme A: Command execution start marker with correlation metadata
                        [Console]::Write("$([char]27)]133;C;kegon.v=1;kegon.sid=$sid;kegon.seq=$seq$([char]7)")
                    } else {
                        # Standard command execution start marker: OSC 133;C
                        [Console]::Write("$([char]27)]133;C$([char]7)")
                    }
                }
                return $line
            }
        }
    } catch { }

    function global:prompt {
        # Preserve $? only for the user's original prompt, never to infer an exit code.
        $lastSuccess = $global:?

        $out = ''
        if ($global:__KegonState.HasReadLine -and $global:__KegonState.Executing) {
            $global:__KegonState.Executing = $false
            if ($env:KEGON_SESSION_ID) {
                try {
                    $status = if ($lastSuccess) { 'success' } else { 'failure' }
                    $seq = $global:__KegonState.Seq
                    $payload = "kegon.v=1;kegon.sid=$($env:KEGON_SESSION_ID);kegon.seq=$seq;kegon.ps=$status"
                    $bytes = [System.Text.Encoding]::UTF8.GetBytes($payload)
                    $b64 = [System.Convert]::ToBase64String($bytes)
                    $out += "$([char]27)]1337;SetUserVar=kegon_status=$b64$([char]7)"
                } catch { }
            }
            # Boundary only: neither $? nor a possibly stale LASTEXITCODE proves the
            # exit code of this input. Success, failure, and compound input are Unknown.
            $out += "$([char]27)]133;D$([char]7)"
        }

        # Prompt start marker: OSC 133;A
        $out += "$([char]27)]133;A$([char]7)"

        # Current working directory: OSC 7
        try {
            $location = $ExecutionContext.SessionState.Path.CurrentLocation
            if ($location.Provider.Name -eq 'FileSystem') {
                $builder = New-Object System.Text.StringBuilder
                foreach ($byte in [System.Text.Encoding]::UTF8.GetBytes($location.ProviderPath.Replace('\', '/'))) {
                    if (($byte -ge 48 -and $byte -le 57) -or ($byte -ge 65 -and $byte -le 90) -or ($byte -ge 97 -and $byte -le 122) -or $byte -in 45, 46, 95, 126, 47, 58) {
                        [void]$builder.Append([char]$byte)
                    } else {
                        [void]$builder.AppendFormat('%{0:X2}', $byte)
                    }
                }
                $path = $builder.ToString()
                if ($path.StartsWith('//')) {
                    $uri = 'file://' + $path
                } else {
                    $uri = 'file://' + $global:__KegonState.HostName + '/' + $path
                }
                $out += "$([char]27)]7;$uri$([char]27)\"
            }
        } catch { }

        # Restore $? before calling original prompt so custom prompts (Oh My Posh, Starship, etc.) can inspect it
        if (-not $lastSuccess) {
            Write-Error 'failure' -ErrorAction Ignore
        }

        # Invoke the original prompt to preserve user's styling and functionality
        try {
            $orig = & $global:__KegonState.OriginalPrompt
            if ($null -ne $orig) {
                $out += -join $orig
            }
        } catch { }

        # Input start marker: OSC 133;B
        $out += "$([char]27)]133;B$([char]7)"
        return $out
    }
} catch { }
