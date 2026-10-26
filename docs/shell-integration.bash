# Source this file from an interactive remote Bash 4.4+ shell.
# PS0 runs after readline has accepted the command, before executing it.
# It never runs while sudo, ssh, an editor, or another child reads input.
[[ $- == *i* ]] || return 0
[[ ${__EVERTY_SHELL_INTEGRATION-} == 1 ]] && return 0
if (( BASH_VERSINFO[0] < 4 || (BASH_VERSINFO[0] == 4 && BASH_VERSINFO[1] < 4) )); then
    printf '%s\n' 'EvertySSH shell integration requires Bash 4.4+' >&2
    return 1
fi
__EVERTY_SHELL_INTEGRATION=1

__everty_emit_command() {
    local entry command history_id
    entry=$(HISTTIMEFORMAT= builtin history 1)
    [[ $entry =~ ^[[:space:]]*([0-9]+)[[:space:]][[:space:]](.*)$ ]] || return 0
    history_id="everty-history:$$:${BASH_REMATCH[1]}"
    command=${BASH_REMATCH[2]}
    [[ -n $command && $command != ' '* ]] || return 0
    command=${command//\\/\\\\}
    command=${command//;/\\x3b}
    command=${command//$'\n'/\\x0a}
    command=${command//$'\r'/\\x0d}
    command=${command//$'\e'/\\x1b}
    command=${command//$'\a'/\\x07}
    printf '\033]633;E;%s;%s\007\033]633;C\007' "$command" "$history_id"
}

# Establish a baseline: HISTCONTROL=ignorespace and ignoredups may leave the
# previous history entry unchanged. Its identifier must not be logged again.
__everty_baseline=$(HISTTIMEFORMAT= builtin history 1)
if [[ $__everty_baseline =~ ^[[:space:]]*([0-9]+) ]]; then
    printf '\033]633;P;EvertyHistoryId=everty-history:%s:%s\007' "$$" "${BASH_REMATCH[1]}"
fi
unset __everty_baseline
PS0='$(__everty_emit_command)'"${PS0-}"
