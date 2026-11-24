# Sourced by the Mac scripts that look for the app's processes. Needs `uid` from the script.
processes_of() {
    ps -axo uid=,pid=,ppid=,comm= | awk -v uid="$uid" -v want="$1" '
        $1 == uid {
            program = $4
            for (i = 5; i <= NF; i++) program = program " " $i
            if (program == want) print $2, $3
        }'
}

is_gone() {
    ! kill -0 "$1" 2> /dev/null
}
