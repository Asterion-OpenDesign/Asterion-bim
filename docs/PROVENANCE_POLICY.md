# Provenance policy

This repository is built so that every file can be explained. This document states the rules and how they are enforced.

## Rules

1. **Original work only.** Every line is written for this project, or comes from a dependency with a licence we comply with
   (listed in `THIRD_PARTY_NOTICES.md`).
2. **Allowed sources of knowledge.** Public standards and specifications, public documentation, textbooks and papers,
   open-source projects under compatible licences (cited in the commit), and files and data that the contributor created
   or has the right to share.
3. **Forbidden sources.** Source code, binaries, SDK files, icons, fonts or data of products we have no right to
   redistribute; the output of any decompiler or disassembler; leaked or confidential material; anything obtained
   under a licence that forbids the use being made of it.
4. **Specifications before code.** A behaviour that is not defined by a public source is written first as a specification in
   `docs/specs/` (template in `docs/specs/TEMPLATE.md`), in original words, citing the public sources it rests on. Code is
   written from the specification.
5. **Separation.** The person or agent who writes a module has not read material excluded by rule 3 about the same area.
   Each module's `provenance/LEDGER.md` entry records its author, the specification it implements, the sources used, and
   a statement of separation.
6. **No claim without evidence.** The project does not call a module independently developed unless its ledger entry shows
   the separation. It does not claim compatibility with any product beyond what a published, reproducible test shows.

## Enforcement

- `tools/provenance_gate.py` runs in CI and as a local hook (`scripts/install-hooks.ps1`). It rejects decompiler artefacts,
  internal names of third-party binaries, local machine paths, personal e-mail addresses, credentials, forbidden file types and
  forbidden directories.
- Maintainers can also run it with a private deny-list of hashed identifiers (`ASTERION_PRIVATE_DENYLIST`), which is never
  stored in the repository.
- Pull requests must state the sources behind the change (see `CONTRIBUTING.md`).

## Limits

A passing gate shows only that known patterns are absent. It is not a legal opinion and not a guarantee.
