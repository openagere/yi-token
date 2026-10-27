# Exporting the request-pipeline oracle corpus

`core/tests/fixtures/oracle/request_pipeline.jsonl` is the behaviour of the
yi-llm request pipeline (branch `refactor/backend-rust`) over the
`protocol_oracle` corpus: 3 client protocols x (1 base + 42 fields x 32
boundary values) x 3 upstream protocols = 12105 cases. It is data; the
12k-line `protocol_oracle_full.txt` snapshot is not committed.

Record schema (one JSON object per line):

| key | meaning |
| --- | --- |
| `case_id` | `<inbound>/<field>=<value> -> <upstream>` |
| `inbound`, `upstream` | `responses` / `openai_chat` / `anthropic` |
| `field`, `value` | the mutated field and the JSON text of its value (`""` for the base case) |
| `request` | the client request body |
| `outcome` | `accepted` or `rejected` (any of option validation, normalisation, upstream check, decode, build failed) |
| `upstream_body` | present when accepted: the upstream JSON yi-llm generated |

Upstream body per case: translation to Anthropic / Chat is
`build_request(...)` with provider `thinking = "medium"` and upstream model
`up`; translation to Responses is the normalised Responses body with
`model = "up"`; same-protocol is the body after `prepare_request` with
`model = "up"`.

## Procedure

yi-llm is read-only. Never `git worktree add`, `checkout` or otherwise change
it. Extract the branch with `git archive` into a directory outside both repos:

```sh
mkdir -p /d/Github/yi-oracle-tmp
git -C D:/Github/yi-llm archive refactor/backend-rust | tar -x -C /d/Github/yi-oracle-tmp
cd /d/Github/yi-oracle-tmp/src-tauri

# the upstream oracle must still pass on the extracted copy
cargo test --test protocol_oracle        # also writes target/protocol_oracle_full.txt (12105 lines)

# one-off exporter: the oracle's own corpus + request-side functions, JSONL output
head -168 tests/protocol_oracle.rs > tests/export_corpus.rs
cat <path-to-yi-token>/scripts/oracle-export-tail.rs >> tests/export_corpus.rs
cargo test --test export_corpus          # writes target/request_pipeline.jsonl

cp target/request_pipeline.jsonl <path-to-yi-token>/core/tests/fixtures/oracle/request_pipeline.jsonl
```

Delete the extracted copy afterwards and confirm yi-llm is unchanged
(`git -C D:/Github/yi-llm status --short`, `git -C D:/Github/yi-llm worktree list`).

`scripts/oracle-export-tail.rs` is the exporter body (not compiled in this repo).

## Comparing

`core/tests/oracle.rs` runs every record through yi-core and compares
accepted/rejected plus the generated upstream JSON as `serde_json::Value`
(key order ignored). Error text and hashes are never compared. Intended
differences are the named rules in `ALLOWED_DIFFS`, each with a reason and a
shape check.
