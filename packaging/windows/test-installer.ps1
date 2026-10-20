param(
    [Parameter(Mandatory)][string]$Version,
    [Parameter(Mandatory)][ValidateSet('x64', 'arm64')][string]$Arch
)
$ErrorActionPreference = 'Stop'
$stem = "tacterm-$Version-windows-$Arch"
$installer = Join-Path $PSScriptRoot "../../target/release-assets/$stem-setup.exe"
$portable = Join-Path $PSScriptRoot "../../target/release-staging/$stem-portable/tacterm.exe"
$destination = Join-Path $env:RUNNER_TEMP ("tacterm-setup-test-" + [guid]::NewGuid())
$log = Join-Path $env:RUNNER_TEMP "$stem-setup.log"
try {
    $process = Start-Process -FilePath $installer -ArgumentList "/CURRENTUSER /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /DIR=`"$destination`" /LOG=`"$log`"" -WindowStyle Hidden -Wait -PassThru
    if ($process.ExitCode -ne 0) { throw "Installer failed: $($process.ExitCode). See $log" }
    $installed = Join-Path $destination 'tacterm.exe'
    if ((Get-FileHash $installed).Hash -ne (Get-FileHash $portable).Hash) {
        throw 'Installed executable differs from portable executable'
    }
    foreach ($relative in @('LICENSE', 'licenses/corcovado/ORIGINAL_LICENSE', 'licenses/sugarloaf/src/font/resources/CascadiaCode/LICENSE')) {
        $sourceLicense = Join-Path (Split-Path $portable) $relative
        $installedLicense = Join-Path $destination $relative
        if ((Get-FileHash $sourceLicense).Hash -ne (Get-FileHash $installedLicense).Hash) {
            throw "Installed license differs from portable: $relative"
        }
    }
    # Use explicit pipes, as in release.mjs: PowerShell does not reliably capture
    # stdout from the Windows GUI-subsystem executable.
    @'
import { execFileSync } from 'node:child_process';
const [executable, version] = process.argv.slice(2);
const reported = execFileSync(executable, ['--version'], { encoding: 'utf8', timeout: 30000 });
if (!reported.includes(version)) throw new Error(`Unexpected version: ${JSON.stringify(reported)}`);
const help = execFileSync(executable, ['--help'], { encoding: 'utf8', timeout: 30000 });
if (!help.includes('--working-dir')) throw new Error('Installed CLI help is incomplete');
console.log(`Installed CLI smoke check passed: ${reported.trim()}`);
'@ | node --input-type=module - $installed $Version
    if ($LASTEXITCODE -ne 0) { throw 'Installed CLI smoke check failed' }
} finally {
    $uninstaller = Join-Path $destination 'unins000.exe'
    if (Test-Path -LiteralPath $uninstaller) {
        $process = Start-Process -FilePath $uninstaller -ArgumentList '/VERYSILENT /SUPPRESSMSGBOXES /NORESTART' -WindowStyle Hidden -Wait -PassThru
        if ($process.ExitCode -ne 0) { throw "Uninstaller failed: $($process.ExitCode)" }
        if (Test-Path -LiteralPath (Join-Path $destination 'tacterm.exe')) { throw 'Uninstaller left tacterm.exe behind' }
    }
}
