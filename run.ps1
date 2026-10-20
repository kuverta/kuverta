# Build and launch the Windows development app with its own data and credentials.
# Usage: .\run.ps1 [-BuildOnly] [-Release] [-ModelDemo] [-Preview] [-DataDir path]
[CmdletBinding()]
param([switch]$BuildOnly, [switch]$Release, [switch]$ModelDemo, [switch]$Preview, [string]$DataDir = '.devdata')

$ErrorActionPreference = 'Stop'
$repoRoot = $PSScriptRoot
$previousPath = $env:Path
$previousInstance = $env:KUVERTA_INSTANCE
$previousDataDir = $env:KUVERTA_DATA_DIR
$fixtureProcess = $null
Push-Location $repoRoot
try {
    if ($Preview) {
        Write-Host 'Open http://127.0.0.1:19326 for the browser preview. Ctrl+C stops it.'
        & node (Join-Path $repoRoot 'tools\preview-models.mjs')
        if ($LASTEXITCODE -ne 0) { throw "Preview exited with code $LASTEXITCODE." }
        return
    }
    $env:Path = (Join-Path $env:USERPROFILE '.cargo\bin') + ';' + $env:Path
    # Optional portable NASM; a system installation on PATH works too.
    $portableNasm = Join-Path $repoRoot '.devtools\nasm-3.02'
    if (Test-Path (Join-Path $portableNasm 'nasm.exe')) {
        $env:Path = $portableNasm + ';' + $env:Path
    }
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        throw 'Rust is missing. Install Rustup, then run rustup toolchain install in this repository.'
    }

    # Tauri serves only ui/. Keep its generated copy identical to the shared UI.
    $uiRoot = [IO.Path]::GetFullPath((Join-Path $repoRoot 'apps\desktop\ui'))
    $generatedRoot = [IO.Path]::GetFullPath((Join-Path $uiRoot 'kuverta-bird'))
    if (-not $generatedRoot.StartsWith($uiRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'The generated UI directory must stay inside apps/desktop/ui.'
    }
    if (Test-Path -LiteralPath $generatedRoot) {
        Remove-Item -LiteralPath $generatedRoot -Recurse -Force
    }
    New-Item -ItemType Directory -Path (Join-Path $generatedRoot 'hosts') -Force | Out-Null
    foreach ($relative in @('core', 'hosts\kuverta')) {
        $source = Join-Path $repoRoot ('kuverta-bird\' + $relative)
        $destination = Join-Path $generatedRoot $relative
        Copy-Item -LiteralPath $source -Destination $destination -Recurse
        foreach ($file in Get-ChildItem -LiteralPath $source -Recurse -File) {
            $copy = Join-Path $destination $file.FullName.Substring($source.Length + 1)
            if ((Get-FileHash -LiteralPath $file.FullName).Hash -ne (Get-FileHash -LiteralPath $copy).Hash) {
                throw "Generated UI differs from source: $copy"
            }
        }
    }

    $buildArgs = @('build', '--locked', '-j', '2', '-p', 'kuverta-desktop')
    $profile = 'debug'
    if ($Release) {
        $buildArgs += '--release'
        $profile = 'release'
    }
    & cargo @buildArgs
    if ($LASTEXITCODE -ne 0) { throw "Kuverta build failed (exit $LASTEXITCODE)." }
    if ($BuildOnly) { return }

    $env:KUVERTA_INSTANCE = 'dev'
    $env:KUVERTA_DATA_DIR = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($DataDir)
    if ($ModelDemo) {
        New-Item -ItemType Directory -Force -Path $env:KUVERTA_DATA_DIR | Out-Null
        $fixtureProcess = Start-Process -FilePath (Get-Command node -ErrorAction Stop).Source `
            -ArgumentList 'tools/model-fixture.mjs' -WorkingDirectory $repoRoot -WindowStyle Hidden -PassThru `
            -RedirectStandardOutput (Join-Path $env:KUVERTA_DATA_DIR 'model-fixture.log') `
            -RedirectStandardError (Join-Path $env:KUVERTA_DATA_DIR 'model-fixture-error.log')
        Start-Sleep -Milliseconds 500
        if ($fixtureProcess.HasExited) { throw "The model fixture could not start; see $env:KUVERTA_DATA_DIR\model-fixture-error.log." }
        Write-Host 'Model demo is running locally; see docs/releasing.md for provider addresses.'
    }
    Write-Host "Opening kuverta - dev with data in $env:KUVERTA_DATA_DIR"
    & (Join-Path $repoRoot "target\$profile\kuverta-desktop.exe")
    if ($LASTEXITCODE -ne 0) { throw "Kuverta exited with code $LASTEXITCODE." }
} finally {
    if ($fixtureProcess -and -not $fixtureProcess.HasExited) { Stop-Process -Id $fixtureProcess.Id }
    $env:Path = $previousPath
    $env:KUVERTA_INSTANCE = $previousInstance
    $env:KUVERTA_DATA_DIR = $previousDataDir
    Pop-Location
}
