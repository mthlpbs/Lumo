<#
.SYNOPSIS
Update Lumo's version, test/build Windows x64, and create EXE, ZIP and Setup.
.EXAMPLE
./tools/package.ps1
.EXAMPLE
./tools/package.ps1 -KeepVersion
.EXAMPLE
./tools/package.ps1 -Version 2026.1108.2 -Installer Inno
#>
[CmdletBinding()]
param(
    [string]$Version,
    [switch]$KeepVersion,
    [switch]$BumpVersion,
    [ValidateSet('Auto','Inno','NSIS')][string]$Installer = 'Auto',
    [ValidateSet('x86_64-pc-windows-gnu','x86_64-pc-windows-msvc')][string]$Target,
    [string]$Date,
    [switch]$SkipTests
)
$ErrorActionPreference = 'Stop'
if (@($Version, $KeepVersion.IsPresent, $BumpVersion.IsPresent | Where-Object { $_ }).Count -gt 1) {
    throw 'Choose only one of -Version, -KeepVersion or -BumpVersion'
}
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$oldLocation = Get-Location
$savedEnvironment = @{}
foreach ($name in @('PATH','CARGO_HOME','RUSTUP_HOME','RUSTUP_TOOLCHAIN','LIBRARY_PATH','NSISDIR')) {
    $savedEnvironment[$name] = [Environment]::GetEnvironmentVariable($name,'Process')
}
$packageLock = $null
function Find-Tool([string]$Name, [string[]]$Candidates = @()) {
    foreach ($candidate in $Candidates) { if (Test-Path -LiteralPath $candidate -PathType Leaf) { return $candidate } }
    $command = Get-Command $Name -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($command) { return $command.Source }
    return $null
}
function Run-Tool([string]$Executable, [string[]]$Arguments) {
    & $Executable @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$(Split-Path $Executable -Leaf) failed (exit $LASTEXITCODE)" }
}
try {
    Set-Location -LiteralPath $root
    $localTools = Join-Path $root '.tools'
    New-Item -ItemType Directory -Force -Path $localTools | Out-Null
    try { $packageLock = [IO.File]::Open((Join-Path $localTools 'package.lock'), 'OpenOrCreate', 'ReadWrite', 'None') }
    catch { throw 'Another packaging run is active in this workspace. Wait for it to finish.' }
    $python = Find-Tool 'python' @((Join-Path $localTools 'python/python.exe'))
    if (-not $python) { $python = Find-Tool 'python3' }
    if (-not $python) { throw 'Python 3.11 or newer is required' }
    # A global Cargo can use MSVC even when the requested output is GNU:
    # build scripts still use the host compiler. Prefer the complete bundled
    # GNU toolchain, not just its target, unless MSVC was explicitly requested.
    $localCargo = Join-Path $localTools 'cargo/bin/cargo.exe'
    $localRust = Join-Path $localTools 'rustup/toolchains/stable-x86_64-pc-windows-gnu'
    $localLinker = Join-Path $localTools 'llvm-mingw-20261020-ucrt-x86_64/bin/x86_64-w64-mingw32-gcc.exe'
    $useLocalRust = ($Target -ne 'x86_64-pc-windows-msvc') -and
        (Test-Path -LiteralPath $localCargo -PathType Leaf) -and
        (Test-Path -LiteralPath (Join-Path $localRust 'bin/rustc.exe') -PathType Leaf) -and
        (Test-Path -LiteralPath $localLinker -PathType Leaf)
    if ($useLocalRust) {
        $cargo = $localCargo
        $env:CARGO_HOME = Join-Path $localTools 'cargo'
        $env:RUSTUP_HOME = Join-Path $localTools 'rustup'
        $env:RUSTUP_TOOLCHAIN = 'stable-x86_64-pc-windows-gnu'
        $env:PATH = "$(Join-Path $localTools 'cargo/bin');$(Join-Path $localTools 'llvm-mingw-20261020-ucrt-x86_64/bin');$env:PATH"
        $env:LIBRARY_PATH = Join-Path $localRust 'lib/rustlib/x86_64-pc-windows-gnu/lib/self-contained'
        $Target = 'x86_64-pc-windows-gnu'
    } else {
        $cargo = Find-Tool 'cargo'
        if (-not $cargo) { throw 'Install Rust/Cargo or provide the complete local .tools Rust and MinGW toolchain' }
    }
    if (-not $Target) {
        $hostInfo = & rustc -vV
        if ($LASTEXITCODE -ne 0) { throw 'Could not identify the Rust host toolchain' }
        $Target = if ($hostInfo -match 'host: x86_64-pc-windows-gnu') { 'x86_64-pc-windows-gnu' } else { 'x86_64-pc-windows-msvc' }
    }
    $inno = Find-Tool 'iscc'
    $nsisCandidates = @(Get-ChildItem -LiteralPath $localTools -Directory -Filter 'nsis*' |
        Sort-Object Name -Descending | ForEach-Object { Join-Path $_.FullName 'compiler/makensis.exe' })
    $nsis = Find-Tool 'makensis' $nsisCandidates
    if ($Installer -eq 'Auto') { $Installer = if ($inno) { 'Inno' } elseif ($nsis) { 'NSIS' } else { throw 'Install Inno Setup 6.5.2+ or NSIS with native amd64 support' } }
    if ($Installer -eq 'Inno' -and -not $inno) { throw 'Inno Setup compiler (iscc) was not found' }
    if ($Installer -eq 'NSIS' -and -not $nsis) { throw 'NSIS compiler (makensis) was not found' }
    if ($Installer -eq 'NSIS') {
        $nsisDirectory = Split-Path -Parent $nsis
        if (Test-Path -LiteralPath (Join-Path $nsisDirectory 'Include/MUI2.nsh')) { $env:NSISDIR = $nsisDirectory }
    }
    $versionTool = Join-Path $PSScriptRoot 'version.py'
    $planArgs = @($versionTool,'plan')
    if ($Version) { $planArgs += @('--version',$Version) }
    if ($KeepVersion) { $planArgs += '--keep-version' }
    if ($BumpVersion) { $planArgs += '--bump-version' }
    if ($Date) { $planArgs += @('--date',$Date) }
    $planJson = & $python @planArgs
    if ($LASTEXITCODE -ne 0) { throw 'Version planning failed' }
    $plan = $planJson | ConvertFrom-Json
    $appVersion = $plan.version
    Write-Host "Building Lumo $appVersion ($Target); installer: $Installer"
    if (-not $SkipTests) {
        Run-Tool $python @('-m','unittest','discover','-s','tools','-p','test_version.py')
        Run-Tool $python @('tools/test_release_workflow.py')
        Run-Tool $cargo @('test','--workspace','--locked','--target',$Target)
    }
    Run-Tool $cargo @('build','-p','lumo','--release','--locked','--target',$Target)
    $built = Join-Path $root "target/$Target/release/lumo.exe"
    $appInfo = (Get-Item -LiteralPath $built).VersionInfo
    if ($appInfo.ProductName.Trim() -ne 'Lumo' -or $appInfo.ProductVersion.Trim() -ne $appVersion) { throw 'Built executable metadata does not match the planned version' }
    $stage = Join-Path $localTools ('package-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $stage | Out-Null
    $zipName = "Lumo-$appVersion-portable.zip"
    $setupName = "Lumo-Setup-$appVersion.exe"
    Copy-Item -LiteralPath $built -Destination (Join-Path $stage 'lumo.exe')
    Copy-Item -LiteralPath (Join-Path $root 'crates/lumo/vendor/webview2/WebView2Loader.dll') -Destination (Join-Path $stage 'WebView2Loader.dll')
    foreach ($license in @('LICENSE','THIRD-PARTY-NOTICES.txt')) { Copy-Item -LiteralPath (Join-Path $root $license) -Destination (Join-Path $stage $license) }
    # ZIP writes directly from the executable and licenses. No dist/Lumo
    # staging directory is created; the Lumo/ folder exists only in the ZIP.
    Run-Tool $python @('tools/package_portable.py','--exe',$built,'--out',(Join-Path $stage $zipName))
    if ($Installer -eq 'Inno') {
        Run-Tool $inno @('/Q',"/DAppVersion=$appVersion","/DExeSource=$built","/O$stage",'installer/lumo.iss')
    } else {
        Run-Tool $nsis @('/V2',"/DEXE=$built","/DOUTDIR=$stage",'installer/lumo.nsi')
    }
    $setupInfo = (Get-Item -LiteralPath (Join-Path $stage $setupName)).VersionInfo
    if ($setupInfo.ProductName.Trim() -ne 'Lumo' -or $setupInfo.ProductVersion.Trim() -ne $appVersion) { throw 'Setup metadata does not match the app version' }
    $unpack = Find-Tool 'innounp'
    if ($Installer -eq 'Inno' -and $unpack) {
        $inspect = Join-Path $stage 'installer-inspection'
        New-Item -ItemType Directory -Path $inspect | Out-Null
        Run-Tool $unpack @('-t','-b',(Join-Path $stage $setupName))
        Run-Tool $unpack @('-x','-b',"-d$inspect",(Join-Path $stage $setupName),'*lumo.exe','*WebView2Loader.dll')
        $payload = @(Get-ChildItem -LiteralPath $inspect -Filter 'lumo.exe' -File -Recurse)
        if ($payload.Count -ne 1 -or (Get-FileHash -LiteralPath $payload[0].FullName).Hash -ne (Get-FileHash -LiteralPath $built).Hash) { throw 'Setup payload differs from the built application' }
        $loaderPayload = @(Get-ChildItem -LiteralPath $inspect -Filter 'WebView2Loader.dll' -File -Recurse)
        if ($loaderPayload.Count -ne 1 -or (Get-FileHash -LiteralPath $loaderPayload[0].FullName).Hash -ne (Get-FileHash -LiteralPath (Join-Path $stage 'WebView2Loader.dll')).Hash) { throw 'Setup WebView2 loader differs from the verified Microsoft binary' }
    }
    Run-Tool $python @('tools/verify_packages.py','--exe',$built,'--portable',(Join-Path $stage $zipName),'--setup',(Join-Path $stage $setupName))
    $checksums = foreach ($name in @('lumo.exe','WebView2Loader.dll',$zipName,$setupName)) {
        '{0}  {1}' -f (Get-FileHash -LiteralPath (Join-Path $stage $name) -Algorithm SHA256).Hash.ToLowerInvariant(),$name
    }
    $checksums | Set-Content -LiteralPath (Join-Path $stage 'SHA256SUMS.txt') -Encoding ascii
    Run-Tool $python @($versionTool,'verify')
    $output = Join-Path $root "dist/$appVersion"
    New-Item -ItemType Directory -Force -Path $output | Out-Null
    # Same-version rebuilds can leave an identical running EXE in place.
    # A changed build gets a fresh version directory by default.
    foreach ($file in Get-ChildItem -LiteralPath $stage -File | Sort-Object @{Expression={if($_.Name -eq 'lumo.exe'){0}else{1}}}) {
        $destination = Join-Path $output $file.Name
        if ((Test-Path -LiteralPath $destination) -and (Get-FileHash -LiteralPath $destination).Hash -eq (Get-FileHash -LiteralPath $file.FullName).Hash) { continue }
        try { Copy-Item -LiteralPath $file.FullName -Destination $destination -Force }
        catch { throw "Cannot write $destination. Close the copy running there or build a new version. $($_.Exception.Message)" }
    }
    Run-Tool $python @($versionTool,'complete')
    Write-Host "Packages ready in $output"
    Get-ChildItem -LiteralPath $output -File | Where-Object { $_.Extension -in @('.exe','.zip') } | Select-Object Name,Length,FullName
} finally {
    if ($packageLock) { $packageLock.Dispose() }
    foreach ($name in $savedEnvironment.Keys) { [Environment]::SetEnvironmentVariable($name,$savedEnvironment[$name],'Process') }
    Set-Location -LiteralPath $oldLocation
}
