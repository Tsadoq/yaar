# Shapes typed by hand

Each file here stands in for something the recorded events in `../claude/` and `../pi/`
never caught. It was typed by hand, not cut from a recording, and its first line says so: a
JSON object whose only field, `hand_made`, says which row it is for and why no recording
serves. The lines after it have the shape of the recordings' lines, and a test reads them
through `yaar_system_tests::recorded::hand_made_hook_events`.

| File | Row | What it stands in for |
| --- | --- | --- |
| `claude-shell-read.jsonl` | LISTEN-01 | Claude Code reading a file with a shell command (`cat notes.txt`): the known limit that such a read is no read step |
