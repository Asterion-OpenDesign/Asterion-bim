# Contributing to ASTERION

Contributions are accepted when their origin can be explained. Read [docs/PROVENANCE_POLICY.md](docs/PROVENANCE_POLICY.md).

## The rule that matters most

Do **not** submit anything that is, or was derived from:

- source code, object code, headers, SDK files, icons, fonts or data of a product you are not licensed to redistribute;
- the output of a decompiler or disassembler run on someone else's binary: pseudocode, routine or symbol names, addresses,
  offsets, string tables, structure layouts;
- leaked, confidential or NDA-covered material;
- code from a project whose licence is incompatible with Apache-2.0, unless a maintainer has approved it first.

If you have studied the internals of a proprietary product in a way that could taint your contribution, tell a maintainer
before working on that area.

## Sign your work

Every commit carries a `Signed-off-by:` line (`git commit -s`), certifying the
[Developer Certificate of Origin 1.1](https://developercertificate.org/).

## Pull requests

1. Open an issue first for anything larger than a small fix.
2. State the sources behind the change: your own design, a public specification (link it), or data you may share.
3. New behaviour comes with a specification in `docs/specs/` and a `provenance/LEDGER.md` entry.
4. Run `python tools/provenance_gate.py` and the tests, and say exactly which commands you ran and what they reported.
5. Keep generated files, build output, personal paths, credentials and real project data out of the change.

Install the local hook with `scripts/install-hooks.ps1`.
