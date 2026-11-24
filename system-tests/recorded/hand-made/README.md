# Shapes typed by hand

Each file here stands in for something the recorded events in `../claude/` and `../pi/`
never caught. It was typed by hand, not cut from a recording, and its first line says so: a
JSON object whose only field, `hand_made`, says why no recording serves. The lines after it
have the shape of the recordings' lines, and a test reads them through
`yaar_system_tests::recorded::hand_made_hook_events`, or, for a shape of pi's (its name
starts with `pi-`), `yaar_system_tests::recorded::hand_made_pi_events`.

| File | What it stands in for |
| --- | --- |
| `claude-shell-read.jsonl` | Claude Code reading a file with a shell command (`cat notes.txt`): the known limit that such a read is no read step |
| `claude-typed-after-self-started.jsonl` | A message the user types after a turn Claude Code started itself (`<task-notification>`): the self-started message carries no review, the typed one does |
| `claude-hook-answers.jsonl` | What Claude Code reads from a hook's output: a review added to the user's message with the line shown under the prompt, and a read allowed. Read through `yaar_system_tests::recorded::hand_made_answers`: each line after the first is `{"answers": <the hook>, "stdout": <the JSON the hook prints>}` |
| `pi-shell-read.jsonl` | pi reading a file with a shell command (`cat notes.txt`): the known limit that such a read is no read step |
| `pi-at-mention.jsonl` | A prompt answered with no tool run, as a prompt of pi naming a file with `@` would be if pi puts the file's text into the message (not verified; checked by hand before a release): no read step, the other known limit |
| `pi-typed-while-working.jsonl` | A message the user types and sends with Enter while pi works: `input` at once with `streamingBehavior` `steer`, as pi did when driven by hand, and the message taken inside the running turn with no new start of the agent |
