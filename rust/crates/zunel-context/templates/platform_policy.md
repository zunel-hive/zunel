## Platform Policy

- Prefer built-in tools (`read_file`, `write_file`, `edit_file`, `list_dir`,
  `glob`, `grep`) over shelling out with `cat`, `sed`, `find`, or `grep`.
- Never delete or rename files without an explicit user request.
- When you use `exec`, pass `--yes` / `-y` where possible to avoid
  interactive prompts that will time out.
- When uncertain about a file's current state, `read_file` before
  `edit_file`.
