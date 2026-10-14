#Requires -Version 5.1
<#
.SYNOPSIS
    Set GH_TOKEN globally (persistent user environment variable) from
    secrets/github-token.txt, wire git to use gh credentials, and optionally
    push the held-back CI workflow.

.DESCRIPTION
    Reads the token from secrets/github-token.txt (gitignored). Use a classic
    token with `repo` and `workflow` scopes.

    - Sets GH_TOKEN for the current session and persistently for the user.
    - Runs `gh auth setup-git` so git uses gh as its credential helper.
    - With -Push, adds and pushes .github/workflows/ci.yml.

.PARAMETER Push
    After setting up, add and push .github/workflows/ci.yml.
#>
param([switch]$Push)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$tokenFile = Join-Path $root 'secrets\github-token.txt'

if (-not (Test-Path -LiteralPath $tokenFile)) {
    throw "Missing $tokenFile - create it and paste your token."
}

$token = (Get-Content -LiteralPath $tokenFile -Raw).Trim()
if (-not $token -or $token -eq 'PASTE_CLASSIC_TOKEN_HERE') {
    throw "Paste your token into $tokenFile first (replace the placeholder)."
}

# Persist GH_TOKEN for the current user and set it for this session.
[Environment]::SetEnvironmentVariable('GH_TOKEN', $token, 'User')
$env:GH_TOKEN = $token
Write-Host "Set GH_TOKEN (user scope, length $($token.Length))."

# Let git use gh's credentials (which now come from GH_TOKEN).
gh auth setup-git 2>$null | Out-Null
gh auth status

if ($Push) {
    Push-Location $root
    try {
        git add .github/workflows/ci.yml
        git commit -m "ci: add build/test workflow"
        git push
    } finally {
        Pop-Location
    }
}
