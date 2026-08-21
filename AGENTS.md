# Working in this repo alongside other agents

This repo is routinely edited by more than one autonomous agent at once —
kaptaind's automated version-bump commits, and separate Claude Code sessions
working different parts of the codebase. That's normal, but it has a real
failure mode: two agents editing the same file at the same time, each
unaware of the other, silently clobbering each other's work.

That happened here, concretely: while one session was mid-edit adding
complexity-tier model routing to `src/groq.rs`, a second live session was
independently editing the same file at the same time, and the change was
only noticed because the file's content had shifted moments after a write.
It was resolved by re-reading the current file state before continuing
rather than assuming the last-known content was still accurate — but that
was a careful catch, not a guarantee. Don't rely on catching it by luck a
second time.

## Before starting non-trivial work

1. `git status` and skim recent commits — is something already in flight?
2. `bash scripts/agent-lock.sh status` — is a live lock held?
3. If both are clear: `bash scripts/agent-lock.sh acquire "<what you're about to do>"`
4. When you're done (or pausing): `bash scripts/agent-lock.sh release`

If `acquire` refuses because a live lock is already held, that's the tool
working as intended — coordinate with whoever holds it (or the user) rather
than overriding it. A lock older than an hour is treated as stale and can be
acquired over; see `scripts/agent-lock.sh` for the exact mechanism.

This is advisory, not enforced by git or CI — it only works if both sides
check it. Treat a `.agent-lock.json` you didn't create as a real signal, not
a formality. And if you're mid-edit on a file and it changes under you
anyway: re-read the current state before writing again. Don't trust a stale
in-memory copy of a file another agent might also be touching.
