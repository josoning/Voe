This project is currently in the 0-to-1 development phase with no existing users. As such, there are no historical technical debts to consider, backward compatibility requirements can be disregarded, and dependencies may be upgraded in a relatively aggressive manner.

# AGENTS.md — Coding & Documentation Guidelines

This file defines project-wide rules that every agent and contributor must follow when
writing code or documentation for the VOE repository.

## Rules

1. **All code and documentation must be written in English.** This includes source code
   comments, commit messages, variable/function names (when descriptive), inline docs,
   markdown files, and any other textual artifact produced for this project.

2. **All code must include sufficient comments.** Non-obvious logic, edge cases, trade-offs,
   and workarounds should be explained with inline comments. Public items (`pub` types,
   traits, functions) must carry rustdoc-style documentation describing purpose, parameters,
   return values, error conditions, and any non-trivial behavior.
