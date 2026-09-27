# nibble completion for fish. Save it with: nibble completion fish > ~/.config/fish/completions/nibble.fish
function __nibble_complete
    set -l current (commandline -ct)
    set -l out (nibble __complete --current=$current -- (commandline -opc) 2>/dev/null)
    switch "$out[1]"
        case :files
            __fish_complete_path $current
        case :dirs
            __fish_complete_directories $current
        case '*'
            printf '%s\n' $out[2..-1]
    end
end
complete -c nibble -f -a '(__nibble_complete)'
