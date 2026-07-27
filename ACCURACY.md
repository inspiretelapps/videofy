# Improving detector accuracy

Written: 2026-07-27. Companion to `STATUS.md` (current state) and
`benchmark/README.md` (how to run the harness).

This is the reasoning behind the calibration work, not a task list. The task
list is in `STATUS.md`.

## Why accuracy cannot improve yet

Every threshold in the system is a number chosen by reasoning, not by
observation:

| where | values |
| --- | --- |
| `src-tauri/src/audio_events.rs` | YAMNet risk thresholds, 0.40–0.65 |
| `src-tauri/src/text_analysis.rs` | rule confidences 0.25–0.72, floor 0.24 |
| both | padding constants that decide how wide each event is |

None has ever been compared against a real film with the unsuitable ranges
marked by hand. So changing a threshold today only swaps one guess for another,
with no way to tell whether recall improved or was quietly destroyed. That is
the entire reason `scan_report` exists.

## The two problems are opposites

"Picking up way too much" is a **precision** problem. What matters for a
six-year-old is **recall** — the scene that gets missed. These pull against each
other: every threshold raised to quiet the noise also risks dropping something
real.

This is why the naive fix — raise everything — is wrong, and why the next
section is the important one.

## The technique that improves both at once

Each detector currently fires independently, so what reaches the review list is
the union of several sources' noise.

Stop treating them as equals:

- **High-precision sources fire alone.** An SDH caption reading `[GUNSHOTS]`, or
  a human-curated guide timestamp, is trustworthy by itself.
- **Weak sources require corroboration.** A loudness jump or a lone YAMNet
  "Screaming" at 0.4 should not raise a card on its own. Independent sources
  agreeing at the same moment should.

That allows the weak detectors to be set *sensitively* — good recall — without
paying for it in noise, because their output only surfaces when something else
agrees.

`fuseEvents` in `src/store.ts` already groups events across sources and nudges
confidence upward on agreement. What is missing is the other half: isolation
should push confidence *down*, and a lone weak clue should not surface at all.

## The loop

Annotate two or three films you know well, then:

```sh
cd src-tauri
cargo run --release --bin scan_report -- "/path/to/Movie.mkv" \
  --annotations ../benchmark/movie.json \
  --json ../benchmark/movie-report.json
```

You get recall, precision, events-per-hour **per source**, and a YAMNet score
histogram. The histogram is the most immediately actionable output. It will show
something like:

```
Gunshot, gunfire     >=0.4: 200    >=0.7: 12
```

If recall holds when the threshold moves to 0.7, that deleted 188 false
positives for free. If recall drops, you have learned the opposite and you keep
0.4. Either way it is now a measurement rather than an opinion.

Repeat per source. Roughly ten minutes a cycle once the annotations exist.

## Where the biggest wins are, in order

1. **YAMNet thresholds.** Likely the loudest source, and the histogram makes it
   a mechanical fix.
2. **Text rule confidences.** Currently guessed. The annotations show which
   phrases actually predict content; the lexicon is a data table, so it is
   editable without touching logic.
3. **Per-category thresholds.** The cost of a miss is not uniform. A missed
   swear word is mild; a missed on-screen death is not. Severity should raise or
   lower the bar for surfacing.

## The part that compounds

Keep / Cut / Mute decisions are already persisted per movie. After a few films
that is a labelled dataset built from real use: which sources and which labels
get acted on versus dismissed, and in what proportion. Re-weighting from that
keeps improving the tool without adding a single detector, and the persistence
half is already built.

## The honest ceiling

Without semantic models this system will get good at *finding candidates with a
traceable evidence trail* and will stay bad at *understanding context* — sarcasm,
menace without volume, a threat delivered calmly. Calibration can plausibly cut
false positives by a large factor. It will not make the tool understand a scene.

Which is why human-curated guides matter more than their position in the
architecture suggests. For a specific family with a known shortlist of films,
they are the only source in the system that knows what a scene *means*.
