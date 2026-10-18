#!/usr/bin/env python3
"""Provenance gate: fails when the tree contains material that must never be published.

It checks text files for (1) decompiler artefacts and internal names of third-party binaries, (2) local machine paths and
personal e-mail addresses, (3) credentials, and (4) forbidden file types. An optional private deny-list of hashed
identifiers (kept OUTSIDE this repository) can be supplied with ASTERION_PRIVATE_DENYLIST.

Usage:  python tools/provenance_gate.py [path ...]      (default: the repository root)
Exit status: 0 clean, 1 findings, 2 usage error. Findings print the rule and location only, never the matched text.
Python standard library only.
"""
import hashlib, json, os, re, subprocess, sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
RULES = json.load(open(os.path.join(ROOT, "provenance", "rules.json"), encoding="utf-8"))
TEXT_RULES = [(r["name"], re.compile(r["regex"]), set(r.get("allow_paths", []))) for r in RULES["text_rules"]]
FORBID_EXT = tuple(e.lower() for e in RULES["forbidden_extensions"])
FORBID_DIRS = set(RULES["forbidden_dirs"])
MAX_BYTES = RULES["max_file_bytes"]
ALLOW_MARK = "provenance-gate: allow"
IDENT = re.compile(r"[A-Za-z_][A-Za-z0-9_]{3,}")


def load_private():
    p = os.environ.get("ASTERION_PRIVATE_DENYLIST")
    if not p:
        return None
    with open(p, encoding="utf-8") as fh:
        return {l.strip().lower() for l in fh if re.fullmatch(r"[0-9a-fA-F]{64}", l.strip())}


def files(paths):
    """Yield (absolute path, path relative to the scanned directory)."""
    for base in paths:
        if os.path.isfile(base):
            yield os.path.abspath(base), os.path.basename(base)
            continue
        base = os.path.abspath(base)
        try:
            out = subprocess.run(["git", "-C", base, "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
                                 capture_output=True, check=True).stdout.decode("utf-8", "replace").split("\0")
            for rel in out:
                if rel:
                    yield os.path.join(base, rel), rel.replace("\\", "/")
            continue
        except Exception:
            pass
        for dp, dn, fn in os.walk(base):
            dn[:] = [d for d in dn if d not in (".git", "target", "node_modules", "__pycache__")]
            for f in fn:
                full = os.path.join(dp, f)
                yield full, os.path.relpath(full, base).replace("\\", "/")


def scan(paths, private=None):
    findings = []
    for p, rel in files(paths):
        parts = rel.split("/")
        if any(x in FORBID_DIRS for x in parts[:-1]):
            findings.append((rel, 0, "forbidden-directory"))
            continue
        if rel.lower().endswith(FORBID_EXT):
            findings.append((rel, 0, "forbidden-file-type"))
            continue
        try:
            size = os.path.getsize(p)
        except OSError:
            continue
        if size > MAX_BYTES:
            findings.append((rel, 0, "file-too-large"))
            continue
        try:
            with open(p, "rb") as fh:
                data = fh.read()
        except OSError:
            continue
        if b"\x00" in data[:4096]:
            continue
        text = data.decode("utf-8", "replace")
        for n, line in enumerate(text.split("\n"), 1):
            if ALLOW_MARK in line:
                continue
            for name, rx, allow in TEXT_RULES:
                if rel in allow:
                    continue
                if rx.search(line):
                    findings.append((rel, n, name))
            if private is not None and rel not in RULES.get("private_allow_paths", []):
                for tok in IDENT.findall(line):
                    if hashlib.sha256(tok.encode()).hexdigest() in private:
                        findings.append((rel, n, "private-denylist"))
                        break
    return findings


def main(argv):
    paths = argv[1:] or [ROOT]
    for p in paths:
        if not os.path.exists(p):
            print("no such path:", p, file=sys.stderr)
            return 2
    private = load_private()
    f = scan(paths, private)
    for rel, n, rule in sorted(set(f)):
        print("%s:%s: %s" % (rel, n or "-", rule))
    print("provenance gate: %d finding(s) in %s%s" % (len(set(f)), ", ".join(os.path.basename(os.path.abspath(p)) or p for p in paths),
                                                    " (private deny-list active)" if private is not None else ""))
    return 1 if f else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
