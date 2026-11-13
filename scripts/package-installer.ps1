param(
    [Parameter(Mandatory=$true)][string]$ReleaseDirectory,
    [Parameter(Mandatory=$true)][string]$EvidenceDirectory
)
$ErrorActionPreference='Stop'
if(-not $IsWindows -or $env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted'){throw 'Build the installer only in isolated GitHub-hosted Windows CI.'}
$repo=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$release=(Resolve-Path -LiteralPath $ReleaseDirectory).Path
$evidence=[IO.Path]::GetFullPath($EvidenceDirectory)
if(Test-Path -LiteralPath $evidence){throw 'Existing installer evidence is preserved.'}
[void][IO.Directory]::CreateDirectory($evidence)
$report=[ordered]@{schema=1;status='running';source_commit=$env:GITHUB_SHA;checks=@();cleanup='not_needed'}
function Require([bool]$Condition,[string]$Name){if(-not $Condition){throw $Name};$script:report.checks+=@{name=$Name;status='passed'}}
function Hash([string]$Path){(Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()}
$stage=Join-Path $env:RUNNER_TEMP ('CodexProxyTray-installer-build-'+[Guid]::NewGuid().ToString('N'))
try {
    $manifest=Get-Content -LiteralPath (Join-Path $release 'CodexProxyTray-update.json') -Raw -Encoding utf8|ConvertFrom-Json
    $package=Get-Content -LiteralPath (Join-Path $repo 'package.json') -Raw -Encoding utf8|ConvertFrom-Json
    $version=[string]$manifest.version
    Require ($manifest.schema -eq 1 -and $manifest.product -ceq 'CodexProxyTray' -and $version -ceq $package.version -and $version -cmatch '^\d+\.\d+\.\d+$' -and $env:GITHUB_SHA -cmatch '^[0-9a-f]{40}$') 'installer_matches_release_source_and_version'
    $zipName="CodexProxyTray-v$version-windows-x64.zip"
    $zip=Join-Path $release $zipName
    $zipHash=Hash $zip
    $sums=@{};foreach($line in Get-Content -LiteralPath (Join-Path $release 'SHA256SUMS.txt')){
        Require ($line -cmatch '^([0-9a-f]{64})  ([A-Za-z0-9_.-]+)$') 'valid_release_checksum_line'
        $name=$Matches[2];Require (-not $sums.ContainsKey($name)) 'unique_release_checksum_entry';$sums[$name]=$Matches[1]
    }
    Require ($sums.Count -eq 2 -and $sums[$zipName] -ceq $zipHash -and $sums['CodexProxyTray-update.json'] -ceq (Hash (Join-Path $release 'CodexProxyTray-update.json'))) 'installer_uses_verified_final_release_zip'
    $compiler=Get-Command makensis.exe -ErrorAction SilentlyContinue|Select-Object -First 1
    $compilerPath=if($compiler){$compiler.Source}else{Join-Path ${env:ProgramFiles(x86)} 'NSIS\makensis.exe'}
    Require (Test-Path -LiteralPath $compilerPath -PathType Leaf) 'nsis_compiler_available'
    $compilerVersion=(& $compilerPath /VERSION|Out-String).Trim()
    Require ($LASTEXITCODE -eq 0 -and $compilerVersion -match '^v?3\.') 'nsis_three_version_verified'
    $report.nsis_version=$compilerVersion
    [void][IO.Directory]::CreateDirectory($stage)
    foreach($name in @('install_application.ps1','install_update.ps1')){
        [IO.File]::WriteAllText((Join-Path $stage $name),(Get-Content -LiteralPath (Join-Path $repo ('src-tauri/src/'+$name)) -Raw -Encoding utf8),[Text.UTF8Encoding]::new($true))
    }
    $setupName="CodexProxyTray-v$version-windows-x64-setup.exe"
    $setup=Join-Path $release $setupName
    Require (-not (Test-Path -LiteralPath $setup)) 'new_installer_output'
    $compilerArguments=@('/V2',"/DRELEASE_DIR=$release","/DOUTPUT_FILE=$setup","/DAPP_VERSION=$version","/DSOURCE_COMMIT=$env:GITHUB_SHA","/DZIP_SHA256=$zipHash","/DENGINE_PATH=$(Join-Path $stage 'install_application.ps1')","/DUPDATE_ENGINE_PATH=$(Join-Path $stage 'install_update.ps1')","/DICON_PATH=$(Join-Path $repo 'src-tauri/icons/icon.ico')",(Join-Path $repo 'scripts/windows-installer.nsi'))
    & $compilerPath @compilerArguments
    Require ($LASTEXITCODE -eq 0 -and (Test-Path -LiteralPath $setup -PathType Leaf)) 'nsis_installer_compiled'
    $fileVersion=[Diagnostics.FileVersionInfo]::GetVersionInfo($setup)
    Require ($fileVersion.ProductVersion -ceq $version -and $fileVersion.FileVersion -ceq "$version.0") 'installer_pe_version_matches_release'
    $stream=[IO.File]::OpenRead($setup);$reader=[IO.BinaryReader]::new($stream)
    try{$stream.Position=0x3c;$peOffset=$reader.ReadUInt32();$stream.Position=$peOffset;Require ($reader.ReadUInt32() -eq 0x4550) 'installer_valid_pe_header';$machine=$reader.ReadUInt16()}finally{$reader.Dispose()}
    $report.installer_machine=('0x{0:x4}' -f $machine)
    Require ($machine -eq 0x14c) 'x86_installer_wrapper_uses_native_x64_powershell_alias'
    $setupHash=Hash $setup
    $text=@($zipName,'CodexProxyTray-update.json',$setupName)|ForEach-Object {"$(Hash (Join-Path $release $_))  $_"}
    [IO.File]::WriteAllText((Join-Path $release 'SHA256SUMS.txt'),($text -join "`n")+"`n",[Text.UTF8Encoding]::new($false))
    $report.version=$version;$report.setup=@{name=$setupName;sha256=$setupHash};$report.payload_zip_sha256=$zipHash
    $report.windows_signature_status=[string](Get-AuthenticodeSignature -LiteralPath $setup).Status
    Require ($report.windows_signature_status -in @('NotSigned','Valid')) 'installer_has_no_invalid_signature'
    $resolved=[IO.Path]::GetFullPath($stage);$boundary=[IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd('\')+'\'
    Require ($resolved.StartsWith($boundary,[StringComparison]::OrdinalIgnoreCase) -and [IO.Path]::GetFileName($resolved) -like 'CodexProxyTray-installer-build-*') 'build_stage_boundary'
    $entries=@(Get-ChildItem -LiteralPath $stage -Force)
    Require ($entries.Count -eq 2 -and @($entries|Where-Object {$_.PSIsContainer -or $_.Name -notin @('install_application.ps1','install_update.ps1') -or ($_.Attributes -band [IO.FileAttributes]::ReparsePoint)}).Count -eq 0) 'build_stage_allowlist'
    foreach($entry in $entries){[IO.File]::Delete($entry.FullName)};[IO.Directory]::Delete($stage)
    $report.cleanup='passed';$report.status='passed'
} catch {$report.status='failed';$report.error=$_.Exception.Message;if(Test-Path -LiteralPath $stage){$report.cleanup='retained_for_diagnostics'};throw}
finally {[IO.File]::WriteAllText((Join-Path $evidence 'installer-package-checks.json'),($report|ConvertTo-Json -Depth 8),[Text.UTF8Encoding]::new($false))}
