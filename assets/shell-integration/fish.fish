# Kegon Shell Integration for fish.
#
# Started as `fish --init-command 'source "$KEGON_FISH_INIT"'`: that runs after config.fish, so the
# user's setup is complete and nothing in $HOME is changed. Speaks the same OSC 133 dialect as the
# PowerShell integration: `C` and `D` carry kegon.v / kegon.sid / kegon.seq, and `D` also carries
# the exit code and kegon.ps.

status is-interactive; or return 0
set -q KEGON_SESSION_ID; or return 0
set -q __kegon_installed; and return 0

# Take the session identity out of the environment before anything the user runs can see it.
set -g __kegon_sid $KEGON_SESSION_ID
set -e KEGON_SESSION_ID
set -e KEGON_FISH_INIT
set -g __kegon_installed 1
set -g __kegon_host '@HOST@'
set -g __kegon_seq 0
set -g __kegon_running 0

# Runs before a command line executes, once per line.
function __kegon_preexec --on-event fish_preexec
    set -g __kegon_running 1
    set -g __kegon_seq (math $__kegon_seq + 1)
    printf '\e]133;C;kegon.v=1;kegon.sid=%s;kegon.seq=%d\a' $__kegon_sid $__kegon_seq
end

# Runs right before each prompt: `$status` is still the exit status of the command line.
#
# fish_prompt itself is left alone, not wrapped: a wrapper has to hand its `$status` on again, and
# there is no way to hand on a multi-element `$pipestatus`. The prompt mark therefore comes from
# this handler, which fish runs before it draws the prompt (and which is not expected to change
# the statuses the prompt reads; the ignored fish tests check that on a real fish). There is no
# input-start mark (B): the state machine does not need one to accept the next command, and the
# user's prompt (and right prompt) stays exactly theirs.
function __kegon_on_prompt --on-event fish_prompt
    set -l last_status $status
    if test $__kegon_running = 1
        set -g __kegon_running 0
        set -l ps success
        test $last_status -ne 0; and set ps failure
        printf '\e]133;D;%d;kegon.v=1;kegon.sid=%s;kegon.seq=%d;kegon.ps=%s\a' \
            $last_status $__kegon_sid $__kegon_seq $ps
    end
    printf '\e]7;file://%s%s\a' $__kegon_host (string escape --style=url -- $PWD)
    printf '\e]133;A\a'
end
