param(
    [Parameter(Mandatory)][string]$ManifestPath,
    [Parameter(Mandatory)][string]$ArchivePath,
    [Parameter(Mandatory)][System.Security.Cryptography.X509Certificates.X509Certificate2]$Certificate,
    [Parameter(Mandatory)][string]$SignaturePath
)
$ErrorActionPreference='Stop'
if(-not $Certificate.HasPrivateKey){throw '所选证书没有可用的签名密钥。'}
$manifest=Get-Content -LiteralPath $ManifestPath -Raw -Encoding UTF8 | ConvertFrom-Json
if($manifest.schema -ne 1 -or $manifest.product -cne 'CodexProxyTray' -or $manifest.zip_sha256 -cne (Get-FileHash -LiteralPath $ArchivePath -Algorithm SHA256).Hash.ToLowerInvariant()){throw '清单未绑定指定 ZIP。'}
Add-Type -AssemblyName System.Security.Cryptography.Pkcs
$content=[Security.Cryptography.Pkcs.ContentInfo]::new([IO.File]::ReadAllBytes($ManifestPath))
$cms=[Security.Cryptography.Pkcs.SignedCms]::new($content,$true)
$signer=[Security.Cryptography.Pkcs.CmsSigner]::new($Certificate)
$signer.DigestAlgorithm=[Security.Cryptography.Oid]::new('2.16.840.1.101.3.4.2.1')
$signer.IncludeOption=[Security.Cryptography.X509Certificates.X509IncludeOption]::EndCertOnly
$cms.ComputeSignature($signer)
$cms.CheckSignature($true)
$signature=$cms.Encode()
$stream=[IO.File]::Open($SignaturePath,'CreateNew','Write','None')
try{$stream.Write($signature,0,$signature.Length);$stream.Flush($true)}finally{$stream.Dispose()}
