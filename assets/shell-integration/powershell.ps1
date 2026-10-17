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
    if (-not (Get-Command -Name prompt -CommandType Function,Filter -ErrorAction SilentlyContinue)) {
        return
    }

    $global:__KegonState = @{
        OriginalPrompt = $function:prompt
        OriginalReadLine = $null
        HasReadLine = $false
        Executing = $false
        HasNative = $false
        HostName = '@HOST@'
    }

    # 4. Attempt to hook PSConsoleHostReadLine for command execution start (OSC 133;C)
    # Must be in isolated try/catch because PSReadLine can throw a terminating error on unsupported hosts
    try {
        if (-not (Get-Module -Name PSReadLine)) {
            Import-Module PSReadLine -ErrorAction Stop
        }
    } catch { }

    try {
        if (Get-Command -Name PSConsoleHostReadLine -ErrorAction SilentlyContinue) {
            $global:__KegonState.OriginalReadLine = $function:PSConsoleHostReadLine
            $global:__KegonState.HasReadLine = $true

            function global:PSConsoleHostReadLine {
                $line = & $global:__KegonState.OriginalReadLine
                if ($null -ne $line -and $line.Trim().Length -gt 0) {
                    $global:__KegonState.Executing = $true
                    $global:__KegonState.HasNative = $false
                    try {
                        $tokens = $null
                        $errors = $null
                        $ast = [System.Management.Automation.Language.Parser]::ParseInput($line, [ref]$tokens, [ref]$errors)
                        if ($null -ne $ast) {
                            $commands = $ast.FindAll({ $args[0] -is [System.Management.Automation.Language.CommandAst] }, $true)
                            foreach ($cmd in $commands) {
                                $cmdName = $cmd.GetCommandName()
                                if ($cmdName) {
                                    $info = Get-Command $cmdName -ErrorAction SilentlyContinue
                                    if ($null -ne $info) {
                                        if ($info.CommandType -eq 'Alias') {
                                            if ($info.ResolvedCommand) {
                                                $info = $info.ResolvedCommand
                                            } else {
                                                $info = Get-Command $info.Definition -ErrorAction SilentlyContinue
                                            }
                                        }
                                        if ($null -ne $info -and $info.CommandType -eq 'Application') {
                                            $global:__KegonState.HasNative = $true
                                            break
                                        }
                                    }
                                }
                            }
                        }
                    } catch { }
                    # Command execution start marker: OSC 133;C
                    [Console]::Write("$([char]27)]133;C$([char]7)")
                }
                return $line
            }
        }
    } catch { }

    function global:prompt {
        # Capture exit status before any statement can overwrite $?
        $lastSuccess = $global:?
        $lastExitCode = $global:LASTEXITCODE

        $out = ''
        if ($global:__KegonState.HasReadLine -and $global:__KegonState.Executing) {
            $global:__KegonState.Executing = $false
            if ($lastSuccess) {
                # Verified success: exit code 0
                $out += "$([char]27)]133;D;0$([char]7)"
            } elseif ($global:__KegonState.HasNative -and $null -ne $lastExitCode -and $lastExitCode -ne 0) {
                # Verified native process failure
                $out += "$([char]27)]133;D;$lastExitCode$([char]7)"
            } else {
                # Non-native failure (cmdlet, script exception, Ctrl+C, etc.): omit code (Unknown)
                $out += "$([char]27)]133;D$([char]7)"
            }
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
