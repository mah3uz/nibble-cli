#compdef nibble
# nibble completion for zsh. Load it after compinit with: eval "$(nibble completion zsh)"
_nibble() {
    local -a lines described
    lines=("${(@f)$(nibble __complete "--current=${words[CURRENT]}" -- "${(@)words[1,CURRENT-1]}" 2>/dev/null)}")
    case ${lines[1]} in
        :files) _files; return ;;
        :dirs) _files -/; return ;;
    esac
    local line
    for line in "${(@)lines[2,-1]}"; do
        [[ -n $line ]] && described+=("${${line%%$'\t'*}//:/\\:}:${line#*$'\t'}")
    done
    (( ${#described} )) && _describe -t nibble nibble described
}
if (( $+functions[compdef] )); then
    compdef _nibble nibble
else
    print -u2 "nibble: completion needs compinit first; put the eval line after it in your .zshrc"
fi
