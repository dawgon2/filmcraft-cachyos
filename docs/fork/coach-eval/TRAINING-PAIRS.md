# Draft training pairs for the grading assistant (AI-PLAN AI3 0c; AI3-PREP part 4)

`training-pairs.jsonl`: one JSON object per line, `{"q", "a", "sources", "kind"}`, generated from
the fork guide by `tools/fork/ai3-training-pairs.py` (rerun it after guide changes):

| kind | from | question form | answer |
|---|---|---|---|
| `practice` | a practice note (docs/guide/practice/) | "How do I <technique>?" | the Revolution way, why, pitfalls |
| `mapping` | a practice note's `resolve:` / `premiere:` header | "In Resolve / Premiere I'd use X. How do I do that in Revolution?" | the same |
| `feature` | a feature section's header and steps | "What does X do?", "What should I use for <when>?" | the header's `does`, then the first steps |

`sources` are compact-header ids (docs/guide/README.md), so an answer can cite them.

**Status: draft.** The questions are templated and some answers are long; before any tuning
(TRAINING.md pipeline on the RX 6700 XT), a person reviews a sample, rewrites unnatural
questions and drops weak pairs. Everything comes from the guide, which is written in our own words
(sources rule, AI3 0a): no third-party text is in the pairs. The test
`coach_ask_tests::training_pairs_are_grounded` checks that every line parses, every source is a
real section and every command an answer names exists.

The advanced eval (`advanced.toml`, 63 questions) is for scoring, not training: keep it out of the
training data.
