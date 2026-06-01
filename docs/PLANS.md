# Plans

Most rustab changes should not need checked-in plans. Use a plan only when the work crosses enough boundaries that future agents need durable context.

## Use a Checked-In Plan For

- protocol changes spanning CLI, mediator, extension code, tests, docs, and release packaging;
- release process redesigns;
- browser support changes with platform-specific install behavior;
- large harness/tooling migrations that affect CI, Nix, scripts, and docs;
- multi-session work that should be resumable without chat history.

## Do Not Check In Plans For

- small bug fixes;
- routine dependency/tooling bumps;
- private operational notes;
- credentials, release-secret handling details, or live debugging transcripts;
- speculative ideas that are not ready to execute.

Use private notes or `.hermes/plans/` for scratch work that should not be part of the public repository history.

## Location and Shape

Checked-in plans live under:

```text
docs/plans/active/<slug>.md
```

When complete, either delete the plan in the finishing commit if it has no durable value, or move it to:

```text
docs/plans/completed/<slug>.md
```

A useful plan includes:

- goal and non-goals;
- current state checked, with paths;
- decisions and open questions;
- implementation tasks in reviewable chunks;
- validation commands and expected artifacts;
- public-boundary considerations;
- progress log with dates only when the log remains useful after completion.

Keep plans concise and executable. If a plan turns into a diary, extract durable decisions into docs/tests/scripts and prune the rest.
