# nibble completion for bash. Load it with: eval "$(nibble completion bash)"
_nibble() {
    local line=${COMP_LINE:0:COMP_POINT} word=${COMP_WORDS[COMP_CWORD]}
    local IFS=$'\n'
    local -a lines
    lines=($(nibble __complete "--line=$line" "--word=$word" 2>/dev/null)) || return
    case ${lines[0]} in
        :files) compopt -o filenames 2>/dev/null; COMPREPLY=($(compgen -f -- "$word")) ;;
        :dirs) compopt -o filenames 2>/dev/null; COMPREPLY=($(compgen -d -- "$word")) ;;
        *) COMPREPLY=($(printf '%s\n' "${lines[@]:1}" | cut -f1)) ;;
    esac
}
complete -F _nibble nibble
