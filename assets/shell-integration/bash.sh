# Kegon Shell Integration for bash.
#
# Started as `bash --rcfile <this file>` in a non-login interactive shell (a login bash is never
# integrated: it ignores --rcfile, and Kegon does not imitate a login shell). bash itself reads the
# system file first (SYS_BASHRC, `/etc/bash.bashrc` on Debian) and then this file in place of
# `~/.bashrc`, so this file sources `~/.bashrc` and installs the hooks last. Nothing in $HOME is
# changed. Speaks the same OSC 133 dialect as the PowerShell integration: `C` and `D` carry
# kegon.v / kegon.sid / kegon.seq, and `D` also carries the exit code and kegon.ps.
#
# Works on bash 3.2 (macOS /bin/bash) and later.

# Take the session identity out of the environment before any user file can see or export it.
__kegon_sid=${KEGON_SESSION_ID-}
unset KEGON_SESSION_ID

# The one startup file `--rcfile` replaces. The system file was already read by bash.
[[ -r ~/.bashrc ]] && builtin source ~/.bashrc

# Only an interactive shell with a session identity is integrated, and only once.
[[ $- == *i* ]] || return 0
[[ -n $__kegon_sid ]] || return 0
[[ -z ${__kegon_installed-} ]] || return 0
__kegon_installed=1

__kegon_host='@HOST@'
__kegon_seq=0
__kegon_ready=      # set once the prompt is drawn and bash waits for a command line
__kegon_running=    # set while a command line that Kegon announced with `C` runs
__kegon_prev_debug=

# Percent-encodes $1 into REPLY (byte-wise, so the result is a valid file URI path).
__kegon_encode_path() {
    local LC_ALL=C
    local s=$1 out= c i
    for ((i = 0; i < ${#s}; i++)); do
        c=${s:i:1}
        case $c in
            [a-zA-Z0-9/._~:-]) out+=$c ;;
            *)
                printf -v c '%%%02X' "'$c"
                out+=$c
                ;;
        esac
    done
    REPLY=$out
}

# First entry of PROMPT_COMMAND: `$?` is still the exit status of the command line.
__kegon_precmd() {
    local __kegon_ret=$?
    # The prompt cycle starts here: whatever PROMPT_COMMAND runs next is not the user's command.
    __kegon_ready=
    if [[ -n $__kegon_running ]]; then
        __kegon_running=
        local __kegon_ps=success
        ((__kegon_ret == 0)) || __kegon_ps=failure
        printf '\033]133;D;%d;kegon.v=1;kegon.sid=%s;kegon.seq=%d;kegon.ps=%s\a' \
            "$__kegon_ret" "$__kegon_sid" "$__kegon_seq" "$__kegon_ps"
    fi
    __kegon_encode_path "$PWD"
    printf '\033]7;file://%s%s\a' "$__kegon_host" "$REPLY"
    printf '\033]133;A\a'
    return "$__kegon_ret"
}

# Last entry of PROMPT_COMMAND: from here on, the next command is the user's. The prompt string is
# final now, so this is where the input-start marker joins it (again each time a PROMPT_COMMAND
# entry of the user's assigns PS1 anew).
__kegon_arm() {
    local __kegon_ret=$?
    [[ $PS1 == *'133;B'* ]] || PS1+='\[\e]133;B\a\]'
    __kegon_ready=1
    return "$__kegon_ret"
}

# Runs before every simple command (DEBUG trap). Only the first one after the prompt is the user's
# command line; a pipeline or a compound command is announced once. Completion functions and
# Kegon's own hooks are ignored.
__kegon_debug() {
    local __kegon_ret=$1 __kegon_cmd=$2
    if [[ -n $__kegon_ready && -z ${COMP_POINT-} ]]; then
        case $__kegon_cmd in
            __kegon_*) ;;
            *)
                __kegon_ready=
                __kegon_running=1
                __kegon_seq=$((__kegon_seq + 1))
                printf '\033]133;C;kegon.v=1;kegon.sid=%s;kegon.seq=%d\a' \
                    "$__kegon_sid" "$__kegon_seq"
                ;;
        esac
    fi
    # Keep a DEBUG trap the user (or a prompt framework) installed before us.
    if [[ -n $__kegon_prev_debug ]]; then
        (exit "$__kegon_ret")
        eval "$__kegon_prev_debug"
    fi
}

# Chain a DEBUG trap that exists already.
__kegon_trap=$(trap -p DEBUG)
if [[ -n $__kegon_trap ]]; then
    eval "set -- $__kegon_trap"
    __kegon_prev_debug=${3-}
fi
unset __kegon_trap
trap '__kegon_debug $? "$BASH_COMMAND"' DEBUG

# PROMPT_COMMAND is a string, or an array from bash 5.1.
if [[ $(declare -p PROMPT_COMMAND 2>/dev/null) == 'declare -a'* ]]; then
    PROMPT_COMMAND=(__kegon_precmd "${PROMPT_COMMAND[@]}" __kegon_arm)
else
    PROMPT_COMMAND="__kegon_precmd"$'\n'"${PROMPT_COMMAND:+$PROMPT_COMMAND$'\n'}__kegon_arm"
fi
