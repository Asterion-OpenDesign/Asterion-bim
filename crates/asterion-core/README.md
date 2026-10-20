# asterion-core

Identity, transactions and diagnostics shared by the engine: element ids and their allocation policy, a transaction layer with commit, rollback, undo and redo, structured failures with a severity, and the internal unit system.

Part of the ASTERION engine. Pure Rust, no unsafe FFI, no network, no operating-system dependencies; the only external
dependency is `serde` (see [THIRD_PARTY_NOTICES.md](../../THIRD_PARTY_NOTICES.md)).

Behavioural rules the code implements are cited as `Spec <id>` and stated in
[docs/specs/engine-rules.md](../../docs/specs/engine-rules.md); open points are in
[docs/open-questions.md](../../docs/open-questions.md). Comments label what each rule rests on: `OBSERVED`, `DOCUMENTED`,
`ASSUMED` or `ENGINE CHOICE`. What the project does and does not claim about the origin of this code is in
[provenance/LEDGER.md](../../provenance/LEDGER.md).

Status: alpha. The API changes without notice. Licence: Apache-2.0.
