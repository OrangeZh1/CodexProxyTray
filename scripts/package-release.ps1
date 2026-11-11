param(
    [Parameter(Mandatory=$true)][string]$PackageDirectory,
    [Parameter(Mandatory=$true)][string]$OutputDirectory,
    [Parameter(Mandatory=$true)][string]$EvidenceDirectory
)
$ErrorActionPreference='Stop'
if(-not $IsWindows -or $env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted'){throw 'Release packaging requires the isolated GitHub-hosted Windows job.'}
$repo=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$package=(Resolve-Path -LiteralPath $PackageDirectory).Path
$output=[IO.Path]::GetFullPath($OutputDirectory)
$evidence=[IO.Path]::GetFullPath($EvidenceDirectory)
foreach($path in @($output,$evidence)){if(Test-Path -LiteralPath $path){throw 'Choose new output and evidence directories; existing records are preserved.'};[void](New-Item -ItemType Directory -Path $path)}
$report=[ordered]@{schema=1;status='running';started_at=[DateTime]::UtcNow.ToString('o');source_commit=$env:GITHUB_SHA;scope='Create and reopen the release ZIP; verify exact files, hashes, version, source and update manifest; no application launch or user configuration access';checks=@();cleanup='not_needed';error=$null}
$tempRoot=[IO.Path]::GetFullPath($env:RUNNER_TEMP)
$stage=Join-Path $tempRoot ('CodexProxyTray-release-'+[Guid]::NewGuid().ToString('N'))
$core=@('CodexProxyTray.exe','codex-proxy-cli.exe','WebView2Loader.dll','LICENSE')
$payload=$core+@('package-receipt.json','QUICKSTART.txt','fluent-icons-license.txt','SHA256SUMS.txt')
function Hash([string]$Path){(Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()}
function Require([bool]$Condition,[string]$Name){if(-not $Condition){throw $Name};$script:report.checks+=@{name=$Name;status='passed'}}
try {
    $receipt=Get-Content -LiteralPath (Join-Path $package 'package-receipt.json') -Raw -Encoding utf8|ConvertFrom-Json
    $npm=Get-Content -LiteralPath (Join-Path $repo 'package.json') -Raw -Encoding utf8|ConvertFrom-Json
    $tauri=Get-Content -LiteralPath (Join-Path $repo 'src-tauri/tauri.conf.json') -Raw -Encoding utf8|ConvertFrom-Json
    $version=[string]$npm.version
    Require ($version -match '^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$' -and $tauri.version -eq $version -and $receipt.version -eq $version) 'version_matches_package_and_application'
    Require ($receipt.schema -eq 1 -and $receipt.product -eq 'CodexProxyTray' -and $receipt.source_commit -eq $env:GITHUB_SHA) 'source_receipt_matches_ci'
    $names=@(Get-ChildItem -LiteralPath $package -Force|Select-Object -ExpandProperty Name)
    Require (@(Compare-Object ($core+@('package-receipt.json','SHA256SUMS.txt')|Sort-Object) ($names|Sort-Object)).Count -eq 0 -and @($receipt.files).Count -eq 4) 'input_package_allowlist'
    foreach($file in $receipt.files){Require ($file.name -in $core -and (Hash (Join-Path $package $file.name)) -eq $file.sha256) ('verified_'+$file.name)}
    $quick=Join-Path $repo 'QUICKSTART.txt'
    Require ((Get-Content -LiteralPath $quick -Raw -Encoding utf8).Contains("CodexProxyTray v$version")) 'offline_guide_matches_version'
    $notes=(Get-Content -LiteralPath (Join-Path $repo "docs/releases/v$version.md") -Raw -Encoding utf8).Trim()
    Require ($notes.Length -gt 0 -and $notes.Length -le 16000) 'release_notes_fit_update_manifest'
    [void](New-Item -ItemType Directory -Path $stage)
    foreach($name in $core+@('package-receipt.json')){Copy-Item -LiteralPath (Join-Path $package $name) -Destination (Join-Path $stage $name)}
    Copy-Item -LiteralPath $quick -Destination (Join-Path $stage 'QUICKSTART.txt')
    Copy-Item -LiteralPath (Join-Path $repo 'docs/fluent-icons-license.txt') -Destination (Join-Path $stage 'fluent-icons-license.txt')
    $hashes=[ordered]@{}
    foreach($name in $core+@('package-receipt.json','QUICKSTART.txt','fluent-icons-license.txt')){$hashes[$name]=Hash (Join-Path $stage $name)}
    $sums=($hashes.Keys|ForEach-Object {"$($hashes[$_])  $_"}) -join "`n"
    [IO.File]::WriteAllText((Join-Path $stage 'SHA256SUMS.txt'),$sums+"`n",[Text.UTF8Encoding]::new($false))
    $hashes['SHA256SUMS.txt']=Hash (Join-Path $stage 'SHA256SUMS.txt')
    $zipName="CodexProxyTray-v$version-windows-x64.zip"
    $zipPath=Join-Path $output $zipName
    Compress-Archive -LiteralPath @($payload|ForEach-Object {Join-Path $stage $_}) -DestinationPath $zipPath -CompressionLevel Optimal
    $archive=[IO.Compression.ZipFile]::OpenRead($zipPath)
    try {
        Require ($archive.Entries.Count -eq $payload.Count -and @(Compare-Object ($payload|Sort-Object) (@($archive.Entries.FullName)|Sort-Object)).Count -eq 0) 'archive_contains_exact_runtime_and_guide'
        foreach($entry in $archive.Entries){
            $stream=$entry.Open();$sha=[Security.Cryptography.SHA256]::Create()
            try {$actual=[Convert]::ToHexString($sha.ComputeHash($stream)).ToLowerInvariant()}finally{$sha.Dispose();$stream.Dispose()}
            Require ($actual -eq $hashes[$entry.FullName]) ('archive_hash_'+$entry.FullName)
        }
    }finally{$archive.Dispose()}
    $manifest=[ordered]@{schema=1;product='CodexProxyTray';version=$version;notes=$notes}
    $manifestPath=Join-Path $output 'CodexProxyTray-update.json'
    [IO.File]::WriteAllText($manifestPath,($manifest|ConvertTo-Json -Depth 4)+"`n",[Text.UTF8Encoding]::new($false))
    Require ((Get-Item -LiteralPath $manifestPath).Length -le 65536) 'update_manifest_within_reader_limit'
    $signature=Get-AuthenticodeSignature -LiteralPath (Join-Path $package 'CodexProxyTray.exe')
    Require ($signature.Status -in @('NotSigned','Valid')) 'no_invalid_windows_signature'
    $report['windows_signature_status']=[string]$signature.Status
    $report['version']=$version;$report['runtime_files']=$receipt.files
    $report['assets']=@($zipName,'CodexProxyTray-update.json'|ForEach-Object {@{name=$_;sha256=Hash (Join-Path $output $_)}})
    [IO.File]::WriteAllText((Join-Path $output 'SHA256SUMS.txt'),(($report.assets|ForEach-Object {"$($_.sha256)  $($_.name)"}) -join "`n")+"`n",[Text.UTF8Encoding]::new($false))
    Require ([IO.Path]::GetFullPath($stage).StartsWith($tempRoot+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase) -and [IO.Path]::GetDirectoryName($stage) -eq $tempRoot) 'temporary_stage_within_runner_temp'
    $items=@(Get-ChildItem -LiteralPath $stage -Force)
    Require ($items.Count -eq $payload.Count -and @($items|Where-Object {$_.PSIsContainer -or $_.Name -notin $payload -or ($_.Attributes -band [IO.FileAttributes]::ReparsePoint)}).Count -eq 0) 'temporary_stage_owned_allowlist'
    Remove-Item -LiteralPath $stage -Recurse -Force
    $report.cleanup='passed';$report.status='passed'
}catch{$report.status='failed';$report.error=$_.Exception.Message;if(Test-Path -LiteralPath $stage){$report.cleanup='retained_for_diagnosis'};throw}
finally{$report['finished_at']=[DateTime]::UtcNow.ToString('o');[IO.File]::WriteAllText((Join-Path $evidence 'release-checks.json'),($report|ConvertTo-Json -Depth 8),[Text.UTF8Encoding]::new($false))}
$report|ConvertTo-Json -Depth 6
