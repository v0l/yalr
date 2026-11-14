# Provider verification

How YALR checks that a user provider serves the model it prices, and where the pass thresholds come from. Config lives under `verification:` in `config.yaml` (see [AUTH.md](../AUTH.md)); the code is in `src/verify/`.

## What a run does

Each priced model is probed against an approved provider an admin marked `reference` for that full model name. A run sends 17 probes to both sides: the 13 fixed probes plus 4 drawn at random from a hidden pool of 33 (`src/verify/probes.rs`). Every probe uses `temperature 0`, `seed 1`, `max_tokens 32`, `logprobs` with `top_logprobs 5`, and `request_extras` (thinking switched off by default). A provider that rejects `logprobs` is retried without it.

Per probe it records:

- **prompt tokens**: `usage.prompt_tokens`, which must equal the reference on every fixed probe;
- **task**: a known answer (arithmetic, a fact, a tool call that must parse);
- **divergence**: the main signal, below;
- **prefix**, **logprob gap** and **top-5 overlap**, as in the first version.

### Divergence

The reference's logprobs say how likely each of its tokens was and what its top 5 alternatives were. Divergence walks the candidate's text along that path and stops at the first token where the two part:

- the same text all the way (or a candidate cut off by `max_tokens`): 0;
- the candidate took one of the reference's top 5 alternatives: the reference's logprob of its own token minus its logprob of the alternative, in nats;
- the candidate took a token outside the reference's top 5, or ended where the reference rated ending outside its top 5: 12 nats (`NOT_IN_TOP`).

Two serving stacks running the same weights disagree only where the model was nearly undecided, so their divergence stays small. A different checkpoint says something the reference model rated unlikely, which shows up as a large divergence. It needs logprobs from the reference only, so it also works for a candidate that does not return them.

## Why not the gap, prefix or overlap

Measured on 46 probes (the 13 fixed plus all 33 pool probes), spark vLLM `qwen3.8-27b` as the reference, OpenRouter candidates pinned to the Alibaba endpoint so both siblings returned logprobs, recorded 14 Nov 2026 (the fixtures in `src/verify/fixtures/`):

| model vs spark 3.8 | logprob gap mean / median / max | top-5 overlap mean | prefix median / min | divergence mean | probes over 4 nats |
|---|---|---|---|---|---|
| 3.8 | 0.034 / 0.008 / 0.230 | 0.81 | 32 / 0 | 0.10 | 0 of 46 |
| 3.6 | 0.041 / 0.010 / 0.270 | 0.69 | 32 / 0 | 1.24 | 4 of 46 |
| 3.5 | 0.065 / 0.018 / 0.474 | 0.65 | 32 / 0 | 1.73 | 7 of 46 |


The logprob gap does not separate them. It is measured over the agreed prefix only, and on that prefix 3.6 and 3.5 are as confident as 3.8 (median 0.008-0.018 nats for all three). Same-model noise reaches 0.23 nats on single probes (`latest-rust`, `symbol-*`), so no single-probe gap threshold works. The mean is kept as a secondary check with `max_logprob_gap: 0.15`, above the highest same-model run mean we saw (0.052) and below nothing we measured, so it only catches gross mismatches.

Prefix agreement does not separate them either. A sibling agrees for 32 tokens on most probes, and the same model across two hosts drops to 0 on open-ended prompts (`cutoff`: "May 2024" vs "December 2024"; `pet-name`, `poem`). The median is kept at `min_prefix_tokens: 8`, which catches a different model family (gemma's median was 2-6). The per-probe minimum (`min_probe_prefix_tokens: 1`) applies to task probes only, where both sides must start the same answer.

Top-5 overlap moves in the right direction (0.81 vs 0.65-0.69) but overlaps between runs and is undefined when the candidate has no logprobs.

## Per-probe data

| probe | set | 3.8 prefix | 3.8 gap | 3.8 top5 | 3.8 div | 3.6 prefix | 3.6 gap | 3.6 top5 | 3.6 div | 3.5 prefix | 3.5 gap | 3.5 top5 | 3.5 div |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `math-17x23` | fixed | 32 | 0.00 | 0.80 | 0.00 | 32 | 0.00 | 0.73 | 0.00 | 32 | 0.00 | 0.73 | 0.00 |
| `capital-france` | fixed | 32 | 0.00 | 0.80 | 0.00 | 32 | 0.01 | 0.80 | 0.00 | 32 | 0.47 | 0.60 | 0.00 |
| `tool-call` | fixed | 32 | 0.02 | 0.76 | 0.00 | 32 | 0.02 | 0.57 | 0.00 | 32 | 0.02 | 0.61 | 0.00 |
| `symbol-gold` | fixed | 32 | 0.02 | 0.80 | 0.00 | 32 | 0.05 | 0.60 | 0.00 | 32 | 0.05 | 0.60 | 0.00 |
| `identity` | fixed | 9 | 0.03 | 0.80 | 1.38 | 32 | 0.03 | 0.63 | 0.00 | 4 | 0.05 | 0.75 | **7.25** |
| `cutoff` | fixed | 0 | - | - | 1.12 | 0 | - | - | **12.00** | 0 | - | - | 0.25 |
| `latest-rust` | fixed | 32 | 0.15 | 0.93 | 0.00 | 3 | 0.10 | 0.80 | **12.00** | 32 | 0.23 | 0.70 | 0.00 |
| `latest-qwen` | fixed | 32 | 0.09 | 0.87 | 0.00 | 32 | 0.19 | 0.73 | 0.00 | 3 | 0.18 | 0.67 | **12.00** |
| `introduce` | fixed | 2 | 0.01 | 0.80 | 0.75 | 2 | 0.03 | 0.60 | 0.75 | 2 | 0.01 | 0.80 | **12.00** |
| `persona-pirate` | fixed | 10 | 0.11 | 0.82 | 0.00 | 0 | - | - | 0.75 | 1 | 0.14 | 0.80 | **12.00** |
| `joke-programmers` | fixed | 32 | 0.04 | 0.75 | 0.00 | 32 | 0.11 | 0.68 | 0.00 | 9 | 0.09 | 0.71 | **6.00** |
| `tip-sleep` | fixed | 12 | 0.10 | 0.88 | 0.12 | 0 | - | - | 0.25 | 1 | 0.31 | 0.80 | 2.75 |
| `slogan` | fixed | 32 | 0.04 | 0.88 | 0.00 | 15 | 0.04 | 0.73 | 2.37 | 0 | - | - | **7.62** |
| `who-made` | pool | 32 | 0.11 | 0.75 | 0.00 | 32 | 0.13 | 0.71 | 0.00 | 4 | 0.03 | 0.75 | 4.00 |
| `name-version` | pool | 32 | 0.02 | 0.76 | 0.00 | 9 | 0.03 | 0.73 | 0.63 | 4 | 0.02 | 0.80 | **5.25** |
| `predecessor` | pool | 32 | 0.07 | 0.82 | 0.00 | 18 | 0.06 | 0.61 | 0.50 | 0 | - | - | 0.38 |
| `metaphor` | pool | 6 | 0.03 | 0.93 | 0.75 | 6 | 0.02 | 0.77 | **12.00** | 4 | 0.02 | 0.75 | 2.63 |
| `pet-name` | pool | 5 | 0.02 | 0.88 | 0.12 | 5 | 0.13 | 0.72 | **12.00** | 0 | - | - | 0.12 |
| `latest-python` | pool | 32 | 0.01 | 0.95 | 0.00 | 32 | 0.09 | 0.85 | 0.00 | 32 | 0.05 | 0.75 | 0.00 |
| `latest-olympics` | pool | 32 | 0.06 | 0.83 | 0.00 | 32 | 0.03 | 0.70 | 0.00 | 1 | 0.02 | 0.60 | 2.87 |
| `knowledge-end` | pool | 32 | 0.11 | 0.73 | 0.00 | 0 | - | - | 2.25 | 0 | - | - | 2.88 |
| `persona-robot` | pool | 6 | 0.09 | 0.77 | 0.50 | 7 | 0.27 | 0.63 | 1.50 | 0 | - | - | 1.62 |
| `math-1` | pool | 32 | 0.00 | 0.87 | 0.00 | 32 | 0.00 | 0.80 | 0.00 | 32 | 0.00 | 0.67 | 0.00 |
| `math-2` | pool | 32 | 0.00 | 0.80 | 0.00 | 32 | 0.00 | 0.73 | 0.00 | 32 | 0.00 | 0.40 | 0.00 |
| `math-3` | pool | 32 | 0.00 | 0.87 | 0.00 | 32 | 0.00 | 0.67 | 0.00 | 32 | 0.00 | 0.67 | 0.00 |
| `math-4` | pool | 32 | 0.00 | 0.80 | 0.00 | 32 | 0.00 | 0.70 | 0.00 | 32 | 0.00 | 0.80 | 0.00 |
| `math-5` | pool | 32 | 0.00 | 1.00 | 0.00 | 32 | 0.00 | 0.80 | 0.00 | 32 | 0.00 | 0.70 | 0.00 |
| `math-6` | pool | 32 | 0.00 | 0.93 | 0.00 | 32 | 0.00 | 0.80 | 0.00 | 32 | 0.00 | 0.67 | 0.00 |
| `math-7` | pool | 32 | 0.00 | 0.80 | 0.00 | 32 | 0.00 | 0.73 | 0.00 | 32 | 0.00 | 0.67 | 0.00 |
| `math-8` | pool | 32 | 0.00 | 0.87 | 0.00 | 32 | 0.00 | 0.80 | 0.00 | 32 | 0.00 | 0.73 | 0.00 |
| `math-9` | pool | 32 | 0.00 | 0.60 | 0.00 | 32 | 0.00 | 1.00 | 0.00 | 32 | 0.00 | 0.80 | 0.00 |
| `math-10` | pool | 32 | 0.00 | 0.80 | 0.00 | 32 | 0.00 | 0.87 | 0.00 | 32 | 0.00 | 0.73 | 0.00 |
| `math-11` | pool | 32 | 0.00 | 0.70 | 0.00 | 32 | 0.00 | 0.90 | 0.00 | 32 | 0.00 | 0.70 | 0.00 |
| `math-12` | pool | 32 | 0.00 | 0.80 | 0.00 | 32 | 0.00 | 0.70 | 0.00 | 32 | 0.00 | 0.60 | 0.00 |
| `capital-1` | pool | 32 | 0.00 | 0.90 | 0.00 | 32 | 0.00 | 0.70 | 0.00 | 32 | 0.03 | 0.70 | 0.00 |
| `capital-2` | pool | 32 | 0.00 | 0.80 | 0.00 | 32 | 0.01 | 0.70 | 0.00 | 32 | 0.13 | 0.70 | 0.00 |
| `capital-3` | pool | 32 | 0.00 | 0.80 | 0.00 | 32 | 0.00 | 0.67 | 0.00 | 32 | 0.00 | 0.73 | 0.00 |
| `capital-4` | pool | 32 | 0.00 | 0.90 | 0.00 | 32 | 0.01 | 0.60 | 0.00 | 32 | 0.28 | 0.60 | 0.00 |
| `capital-5` | pool | 32 | 0.00 | 0.60 | 0.00 | 32 | 0.00 | 0.60 | 0.00 | 32 | 0.09 | 0.60 | 0.00 |
| `capital-6` | pool | 32 | 0.05 | 0.70 | 0.00 | 32 | 0.05 | 0.60 | 0.00 | 32 | 0.01 | 0.60 | 0.00 |
| `symbol-1` | pool | 32 | 0.00 | 1.00 | 0.00 | 32 | 0.01 | 0.40 | 0.00 | 32 | 0.02 | 0.40 | 0.00 |
| `symbol-2` | pool | 32 | 0.04 | 0.60 | 0.00 | 32 | 0.03 | 0.40 | 0.00 | 32 | 0.05 | 0.40 | 0.00 |
| `symbol-3` | pool | 32 | 0.23 | 0.80 | 0.00 | 32 | 0.24 | 0.60 | 0.00 | 32 | 0.25 | 0.40 | 0.00 |
| `symbol-4` | pool | 32 | 0.00 | 0.60 | 0.00 | 32 | 0.01 | 0.60 | 0.00 | 32 | 0.02 | 0.40 | 0.00 |
| `symbol-5` | pool | 32 | 0.01 | 0.60 | 0.00 | 32 | 0.01 | 0.40 | 0.00 | 32 | 0.00 | 0.40 | 0.00 |
| `symbol-6` | pool | 32 | 0.06 | 1.00 | 0.00 | 32 | 0.01 | 0.40 | 0.00 | 32 | 0.00 | 0.60 | 0.00 |

Prefix is in tokens (32 means identical text). Gap and top-5 are over the agreed prefix. A bold divergence is over the 4 nat line.

The probes that separate siblings are version-sensitive ones: the knowledge cutoff (3.6 says "October 2023"; both 3.8 hosts say "December 2024" on one wording and "May 2024" on another), the newest Rust release it knows (3.6: 1.83.0, 3.8: 1.85.0), the newest Qwen model (3.5 names itself), self-identification under a neutral system prompt (3.5: "I am Qwen3.5"), and style on short open prompts (persona, joke, slogan). Arithmetic and capitals are identical across siblings and only catch other families and broken stacks.

Same-model runs also differ: every same-model run we measured had a divergence over 1 nat on 2-5 probes, and one probe went over 4 nats once (a second cutoff wording, "Up to which month and year does your training data go?", which was dropped from the set for that reason). That is why one hard divergence is allowed.

## Pass rule

A model passes when all of these hold (defaults in brackets):

| Check | Setting | Default | Same model (worst seen) | Nearest sibling (best seen) |
|---|---|---|---|---|
| fixed-probe prompt tokens equal | | exact | 0 mismatches | 0 (siblings share a tokenizer) |
| task probes pass | | all | all | all |
| probes with divergence over `hard_divergence_nats` | `max_hard_divergences` | 1 over 4.0 nats | 0 in 17-probe draws, 0 in 46-probe runs | 2 |
| mean divergence | `max_mean_divergence` | 1.0 nats | 0.60 | 1.04 (46 probes), 1.65 (17-probe draws) |
| mean logprob gap, when the candidate returns logprobs | `max_logprob_gap` | 0.15 nats | 0.052 | not separating |
| median prefix | `min_prefix_tokens` | 8 tokens | 12 | not separating |
| smallest prefix on a task probe, with candidate logprobs | `min_probe_prefix_tokens` | 1 token | 32 | not separating |

Over every possible 4-probe draw from the pool (40,920 draws) with the recorded responses, the sibling 3.6 has at least 2 hard divergences and a mean of at least 1.65 nats, and 3.5 at least 6 and 3.52 nats, while the second 3.8 host has none and a mean of at most 0.28. Two earlier recordings of the same probes gave the second 3.8 host a mean of at most 0.60. The 13 fixed probes alone give 3.6 two hard divergences (`cutoff`, `latest-rust`), which is enough to fail it whatever the draw.

When the reference returns logprobs and the candidate does not, a run that clears every check is recorded as `needs_review` and the provider stays pending with "needs admin review". Without the candidate's logprobs the gap and per-token checks cannot run, so this is an admin's call. An approved provider that drops logprobs is not suspended for it.

## Real runs

Reference spark vLLM `qwen3.8-27b`, candidates on OpenRouter with default provider routing, 14 Nov 2026, `cargo run --example verify_real` with `VERIFY_ALL=1` (all 46 probes):

| candidate | outcome | tokens differ | hard divergences | mean divergence | logprob gap | median prefix | candidate logprobs |
|---|---|---|---|---|---|---|---|
| `qwen/qwen3.8-27b` | passed | 0/13 | 0 | 0.20 | 0.027 | 32 | yes |
| `qwen/qwen3.6-27b` | failed | 0/13 | 4 | 1.04 | - | 32 | no |
| `qwen/qwen3.5-27b` | failed | 0/13 | 6 | 1.52 | 0.067 | 32 | no |
| `google/gemma-3-27b-it` | failed | 13/13 | 16 | 4.15 | - | 6 | no |
| `meta-llama/llama-3.3-70b-instruct` | failed | 13/13 | 14 | 2.89 | - | 32 | no |

Three normal draws each (13 fixed + 4 random): 3.8 passed 3 of 3 (0 hard, mean 0.24-0.40); 3.6 failed 3 of 3 (2-3 hard, mean 1.99-2.76); 3.5 failed 3 of 3 (6-7 hard); gemma and llama failed 3 of 3 on token counts and 6-9 hard divergences.

To rerun:

```bash
REFERENCE_URL=http://spark:8001/v1 REFERENCE_MODEL=qwen3.8-27b \
CANDIDATE_URL=https://openrouter.ai/api/v1 CANDIDATE_KEY=... CANDIDATE_MODEL=qwen/qwen3.6-27b \
VERIFY_ALL=1 cargo run --example verify_real
```

## Limits

- The thresholds come from one reference and one model family. A new reference should be checked the same way before its pool is trusted: run `verify_real` against a second host of the same model and against the previous release.
- A sibling with the same training data cutoff and the same answers to version questions would only be caught on style. Add probes whose answers differ between the releases you care about to `POOL` in `src/verify/probes.rs`; the fixture tests in `src/verify/fixture_tests.rs` keep the separation from regressing.
