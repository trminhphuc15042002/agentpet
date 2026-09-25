# Show how far this fork is behind ntd4996/agentpet and list the new commits.
#
#   pwsh scripts/upstream-check.ps1
#
# Exits with code 1 when upstream has new commits, so it can gate a script.
# (The repo also has a scheduled "Upstream watch" workflow that opens an issue.)

$ErrorActionPreference = "Stop"

git remote get-url upstream *> $null
if ($LASTEXITCODE -ne 0) {
    Write-Host "adding upstream remote..."
    git remote add upstream https://github.com/ntd4996/agentpet.git
}

git fetch --no-tags upstream main | Out-Null
$behind = [int](git rev-list --count HEAD..upstream/main)

if ($behind -eq 0) {
    Write-Host "Up to date with upstream/main. Nothing to pull."
    exit 0
}

Write-Host "Upstream/main has $behind new commit(s):"
Write-Host ""
git log --oneline --no-decorate HEAD..upstream/main
Write-Host ""
Write-Host "Pull them in:"
Write-Host "  git merge upstream/main                 # or"
Write-Host "  git rebase upstream/main <feature-branch>"
exit 1
