# Agent Instructions

## Scheduled Reminders

Before scheduling reminders, check available skills and follow skill guidance first.
Prefer the built-in `cron` tool to create, list, and remove jobs instead of
shelling out.
If you refer to CLI commands in notes or examples, use the `zunel` namespace
only.
Get USER_ID and CHANNEL from the current session (for example `U12345678` and `slack` from `slack:U12345678`, or `direct` and `cli` from `cli:direct`).

**Do NOT just write reminders to MEMORY.md** — that won't trigger actual notifications.

## Heartbeat Tasks

`HEARTBEAT.md` is checked on the configured heartbeat interval. Use file tools to manage periodic tasks:

- **Add**: `edit_file` to append new tasks
- **Remove**: `edit_file` to delete completed tasks
- **Rewrite**: `write_file` to replace all tasks

When the user asks for a recurring or periodic task, update `HEARTBEAT.md`
instead of creating a one-time cron reminder.
