# Demo record
Status: fixture for scripts/parity-check.sh --self-test.

| Row id | Upstream behavior | Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| DEMO-ONE | none | does one | implemented | `one_works` |
| DEMO-TWO | none | does two | automated-verified (job `demo`) | `two_works` |
| DEMO-THREE | none | later | deferred (M2) | none |
| DEMO-FOUR | `a\|b` is text | pipe in a cell | implemented (data only) | `four_works` |
| DEMO-FIVE | none | rejected today | not reproduced; implemented | none |

## Divergences
| Row id | Upstream behavior | Stonecrab behavior | Reference | Regression test id |
| --- | --- | --- | --- | --- |
| DEMO-ONE | x | y | V1 section 1 | `one_works` |
