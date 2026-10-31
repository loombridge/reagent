# Installs the Windows installer silently, checks that everything in the
# manifest is there, uninstalls it, and checks that everything is gone.
# Usage: installer/test-windows.ps1 <installer.msi> <manifest.txt>
param([string]$Msi, [string]$Manifest)
$ErrorActionPreference = 'Stop'
$Msi = (Resolve-Path $Msi).Path
$failed = $false

function Run-Msiexec([string]$what) {
    $process = Start-Process msiexec -ArgumentList "$what /qn /norestart" -Wait -PassThru
    if ($process.ExitCode -ne 0) { throw "msiexec $what exited with $($process.ExitCode)" }
}

Run-Msiexec "/i `"$Msi`" /l*v install.log"
foreach ($path in Get-Content $Manifest) {
    if (-not (Test-Path -LiteralPath $path)) { Write-Host "missing after install: $path"; $failed = $true }
}

Run-Msiexec "/x `"$Msi`" /l*v uninstall.log"
foreach ($path in Get-Content $Manifest) {
    if (Test-Path -LiteralPath $path) { Write-Host "left after uninstall: $path"; $failed = $true }
}
if ($failed) { exit 1 }
