#Requires -Version 5.1
<#
.SYNOPSIS
    Create/trust a local development code-signing certificate and sign built
    executables so Windows Application Control (Smart App Control / WDAC) does
    not block them.

.DESCRIPTION
    Some Windows machines block unsigned binaries with error 4551
    ("An Application Control policy has blocked this file"). This script:
      1. creates a self-signed code-signing certificate in CurrentUser\My,
      2. trusts it in CurrentUser\Root and CurrentUser\TrustedPublisher,
      3. signs every .exe under the given directories with Authenticode.

    This is for local development only. Production releases require a
    certificate from a trusted CA.

.PARAMETER Paths
    Directories to sign. Defaults to the debug/release target directories.
#>
param(
    [string[]]$Paths,
    [string]$Subject = 'CN=MeshCloud Dev Signing'
)

$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent $PSScriptRoot
if (-not $Paths -or $Paths.Count -eq 0) {
    $Paths = @(
        (Join-Path $root 'target\debug'),
        (Join-Path $root 'target\debug\deps'),
        (Join-Path $root 'target\release'),
        (Join-Path $root 'target\release\deps')
    )
}

# 1. Certificate.
$cert = Get-ChildItem Cert:\CurrentUser\My -CodeSigningCert |
    Where-Object { $_.Subject -eq $Subject } | Select-Object -First 1
if (-not $cert) {
    Write-Host "Creating code-signing certificate: $Subject"
    $cert = New-SelfSignedCertificate `
        -Type CodeSigningCert `
        -Subject $Subject `
        -FriendlyName 'MeshCloud Dev Signing' `
        -CertStoreLocation 'Cert:\CurrentUser\My' `
        -KeyUsage DigitalSignature `
        -KeyAlgorithm RSA -KeyLength 2048 `
        -NotAfter (Get-Date).AddYears(5) `
        -TextExtension @('2.5.29.37={text}1.3.6.1.5.5.7.3.3')
} else {
    Write-Host "Using existing certificate: $($cert.Thumbprint)"
}

# 2. Trust for the current user.
foreach ($name in @('Root', 'TrustedPublisher')) {
    $store = New-Object System.Security.Cryptography.X509Certificates.X509Store($name, 'CurrentUser')
    $store.Open('ReadWrite')
    $present = $store.Certificates | Where-Object { $_.Thumbprint -eq $cert.Thumbprint }
    if (-not $present) { $store.Add($cert) }
    $store.Close()
    Write-Host "Trusted in CurrentUser\$name"
}

# 3. Sign executables and dynamic libraries (proc-macros load as DLLs).
$signed = 0
foreach ($dir in $Paths) {
    if (-not (Test-Path -LiteralPath $dir)) { continue }
    Get-ChildItem -LiteralPath $dir -File |
        Where-Object { $_.Extension -in '.exe', '.dll' } |
        ForEach-Object {
            $result = Set-AuthenticodeSignature -FilePath $_.FullName -Certificate $cert -HashAlgorithm SHA256
            if ($result.Status -eq 'Valid') { $signed++ }
        }
}

# 3b. Sign cargo build-script executables (target/*/build/**/*.exe).
foreach ($profile in @('debug', 'release')) {
    $buildDir = Join-Path $root "target\$profile\build"
    if (-not (Test-Path -LiteralPath $buildDir)) { continue }
    Get-ChildItem -LiteralPath $buildDir -Recurse -File -Filter *.exe |
        ForEach-Object {
            $result = Set-AuthenticodeSignature -FilePath $_.FullName -Certificate $cert -HashAlgorithm SHA256
            if ($result.Status -eq 'Valid') { $signed++ }
        }
}
Write-Host "Signed $signed file(s)."
