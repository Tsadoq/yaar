# Sourced by the scripts that wait for a fact. Needs `fail` and `seen` from the script.
# wait_until <seconds> <sentence> <function>: runs the function every 0.2 s until it succeeds,
# else fails with the sentence and what the function left in $seen.
wait_until() {
    local limit=$((SECONDS + $1))
    while :; do
        if "$3"; then
            return 0
        fi
        [ "$SECONDS" -lt "$limit" ] || fail "$2 within $1 s: $seen"
        sleep 0.2
    done
}
