---
name: gauntlet
description: Loop work against a frozen quality bar with a separated critic until it clears — builder and checker never the same pass, machine gates outrank every rubric, and the run stops at the bar or the cap, never in between
tools: read_file, list_dir, grep, write_file, edit_file, run_command, run_tests, read_skill
---

# The gauntlet

A prompt produces one attempt. A gauntlet produces attempts until one
clears a bar that was fixed before the first attempt existed. The
difference between the two is not effort — it is that the gauntlet has
a **gate that can fail the work**, and a **critic that is not the
builder**. Without the first you have an agent agreeing with itself on
repeat; without the second you have a grader marking their own exam.

## When a gauntlet is worth running

All four, or use a single prompt instead:

1. The result can be **rejected automatically or against written
   criteria** — a test, `lisa dev check`, a build, or a rubric strict
   enough that two readers score it the same.
2. "Done" is **objective**. If quality is a matter of taste, a person
   decides, not a loop.
3. The work is **end-to-end doable here** — no step that hands half the
   job back to the person mid-loop.
4. The task deserves more than one attempt. A gauntlet on a one-liner
   is ceremony.

## The protocol

**0. Freeze the bar.** Three to six criteria, each one checkable, no
soft passes. Write them down BEFORE any building — in the task, or in a
`GAUNTLET.md` beside the work. The bar does not move during the run:
discovering the bar was wrong ends this gauntlet and starts a new one,
recorded as such. A bar that drifts mid-run is how half-done work gets
promoted to done.

**1. Build.** One focused attempt at the piece in front of you. Split
big work into pieces that can be judged alone — a criterion that can
only be checked "at the end" is a criterion you will check never.

**2. Gate first.** Run the machine checks before any opinion: the
project's tests, `lisa dev check` for an app, the linters. **A red gate
ends the pass** — no rubric, however eloquent, outranks a failing test.
Fix and re-gate before scoring anything.

**3. Criticize — as the critic, not the builder.** Score the work
against each criterion, 1–10, brutally. The critic pass must not defend
the builder's choices: it reads the WORK, not the intent. Name the
weakest criterion and exactly why it is weak. On Lisa the strongest
form of this is a second agent with reviewer instructions; the minimum
acceptable form is a separate pass that quotes the criteria back and
scores them one by one before any prose.

**4. Decide.** Every criterion at its threshold (8+ unless the bar says
otherwise) → done: say `CLEARED`, then stop. Otherwise fix the WEAKEST
criterion first — not the most interesting one — and go again.

**5. Stop honestly.** Two exits, both mandatory to declare:
- `CLEARED` — the bar is met, every gate green.
- `CAPPED` — the iteration cap hit (default 8; set it at freeze time).
  Report what cleared, what did not, and why — a capped run that reads
  like a cleared one is the worst outcome this protocol has.

## State between passes

Keep the running record in `GAUNTLET.md` next to the work: the frozen
bar, what each pass tried, each pass's scores, what is next. A pass
that repeats an attempt already recorded has stopped iterating and
started spinning — the record is what makes tomorrow's run a resume
instead of a restart.

## Cost, reported

At stop, state passes used and the accept/reject tally. The metric that
matters is **cost per accepted change**: a gauntlet that produced ten
attempts and kept four did the review work it was meant to save. If the
tally runs below half over a few runs, the bar or the split is wrong —
say so instead of running it again.

## What this protocol cannot do

- It cannot override a guard refusal or any machine gate — those are
  not criteria, they are the floor (ADR-0029/0030).
- It cannot grade taste. A criterion like "feels polished" is not a
  criterion; rewrite it as something checkable or take it to a person.
- It does not schedule itself. A gauntlet is one run with a cap;
  putting one on a timer is a separate decision made outside the loop.
