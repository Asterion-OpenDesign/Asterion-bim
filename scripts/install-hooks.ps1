# Installs a local pre-commit hook that runs the provenance gate on the working tree.
# Optional: set ASTERION_PRIVATE_DENYLIST (a file of hashed identifiers kept outside the repository).
$ErrorActionPreference = 'Stop'
$root = (git rev-parse --show-toplevel).Trim()
$hook = Join-Path $root '.git\hooks\pre-commit'
$body = @'
#!/bin/sh
python tools/provenance_gate.py || python3 tools/provenance_gate.py || exit 1
'@
[IO.File]::WriteAllText($hook, ($body -replace "`r`n", "`n"), (New-Object Text.UTF8Encoding($false)))
Write-Host "installed $hook"
