# Install Notepad Studio on Windows: copy the release exe into a Programs
# folder and write the registry entries that make it appear in Explorer's
# "Open With" submenu (and "Open With > Choose another app" picker) for
# every text-ish file type we can handle.
#
# Two install scopes:
#   -Scope User    : install to %LOCALAPPDATA%\Programs\Notepad Studio\
#                    (HKCU registry — no elevation needed; default)
#   -Scope Machine : install to %ProgramFiles%\Notepad Studio\
#                    (HKLM registry — requires elevation)
#
# Run from PowerShell:
#   .\scripts\install-windows.ps1                 # per-user install
#   .\scripts\install-windows.ps1 -Scope Machine  # all-users (run as admin)
#   .\scripts\install-windows.ps1 -Uninstall      # remove the install
#
# This script does NOT install a shell extension or context menu DLL — it
# only wires the standard Windows "Open With" lookup chain (Applications
# subkey + SupportedTypes). Users who want a permanent "Edit with Notepad
# Studio" entry should pick us from "Open With" and tick "Always use this
# app" for the file type they care about.

[CmdletBinding()]
param(
    [ValidateSet("User", "Machine")]
    [string]$Scope = "User",
    [switch]$Uninstall
)

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
$exeSrc   = Join-Path $repoRoot "target\release\notepad-studio.exe"

# Resolve install location + registry root from scope.
if ($Scope -eq "Machine") {
    # HKLM\Software\Classes is mirrored into HKCR for every user. Needs
    # elevation — Set-ItemProperty into HKLM throws "Access is denied" for
    # a non-admin PowerShell session.
    $installDir = Join-Path $env:ProgramFiles "Notepad Studio"
    $classesRoot = "HKLM:\Software\Classes"
    $needsAdmin = $true
} else {
    # HKCU\Software\Classes is mirrored into HKCR for the current user only
    # — no elevation needed. The standard "user-scope install" location for
    # apps on Windows 10/11 is %LOCALAPPDATA%\Programs\<App> (matches what
    # VS Code, Slack, Discord etc. all do).
    $installDir = Join-Path $env:LOCALAPPDATA "Programs\Notepad Studio"
    $classesRoot = "HKCU:\Software\Classes"
    $needsAdmin = $false
}

if ($needsAdmin) {
    $current = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
    if (-not $current.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        Write-Error "Scope=Machine requires an elevated PowerShell session. Right-click PowerShell -> Run as administrator, then re-run this script."
        exit 1
    }
}

$exeName  = "notepad-studio.exe"
$exeDst   = Join-Path $installDir $exeName
$appKey   = Join-Path $classesRoot "Applications\$exeName"

# Extensions we want to appear in "Open With" for. Kept in sync with the
# Linux MimeType list in assets/notepad-studio.desktop and the macOS
# CFBundleDocumentTypes list in scripts/bundle-mac.sh — when adding new
# file types, update all three.
$extensions = @(
    ".txt", ".text", ".log", ".md", ".markdown", ".mkd", ".mdown",
    ".csv", ".tsv",
    ".json", ".jsonc", ".json5",
    ".xml", ".svg", ".xhtml", ".xsl", ".xslt", ".plist",
    ".html", ".htm", ".shtml",
    ".css", ".scss", ".sass", ".less",
    ".js", ".mjs", ".cjs", ".jsx",
    ".ts", ".tsx",
    ".rs",
    ".py", ".pyi", ".pyw",
    ".c", ".h",
    ".cpp", ".cxx", ".cc", ".hpp", ".hxx", ".hh",
    ".cs", ".csx",
    ".java",
    ".go",
    ".rb", ".rake", ".gemspec",
    ".php", ".phtml",
    ".swift",
    ".kt", ".kts",
    ".lua",
    ".sh", ".bash", ".zsh", ".ksh", ".fish",
    ".ps1", ".psm1", ".psd1",
    ".yaml", ".yml",
    ".toml",
    ".ini", ".conf", ".cfg", ".env", ".editorconfig", ".gitignore",
    ".sql",
    ".rtf", ".doc", ".docx", ".odt",
    ".patch", ".diff",
    ".tex", ".bib",
    ".r",
    ".dart",
    ".vue", ".svelte"
)

function Remove-Install {
    Write-Host "==> Uninstalling Notepad Studio ($Scope scope)" -ForegroundColor Cyan

    # Stop any running instance so the exe isn't locked.
    Get-Process notepad-studio -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue

    if (Test-Path $appKey) {
        Write-Host "==> Removing registry: $appKey"
        Remove-Item -Path $appKey -Recurse -Force
    }

    # Detach our exe from per-extension OpenWithProgids so the Open With
    # entry stops appearing. Explorer caches the merged list elsewhere too
    # (FileExts\<ext>\OpenWithList) — those are user-driven "Always use…"
    # decisions and we don't touch them.
    foreach ($ext in $extensions) {
        $extKey = Join-Path $classesRoot $ext
        if (Test-Path $extKey) {
            $owl = Join-Path $extKey "OpenWithProgids"
            if (Test-Path $owl) {
                Remove-ItemProperty -Path $owl -Name "Applications\$exeName" -ErrorAction SilentlyContinue
            }
        }
    }

    if (Test-Path $installDir) {
        Write-Host "==> Removing install directory: $installDir"
        Remove-Item -Path $installDir -Recurse -Force
    }

    Write-Host ""
    Write-Host "Uninstalled. You may need to log out or run:"
    Write-Host "  ie4uinit.exe -show"
    Write-Host "to refresh Explorer's Open With cache."
}

function Install-Install {
    if (-not (Test-Path $exeSrc)) {
        Write-Host "==> target\release\notepad-studio.exe not found — running cargo build --release"
        Push-Location $repoRoot
        try {
            & cargo build --release
            if ($LASTEXITCODE -ne 0) { throw "cargo build --release failed" }
        } finally {
            Pop-Location
        }
    }
    if (-not (Test-Path $exeSrc)) {
        throw "Build did not produce $exeSrc"
    }

    # Kill any running instance so we can overwrite the exe.
    Get-Process notepad-studio -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 300

    Write-Host "==> Installing exe to $exeDst"
    New-Item -Path $installDir -ItemType Directory -Force | Out-Null
    Copy-Item -Path $exeSrc -Destination $exeDst -Force

    Write-Host "==> Writing registry: $appKey"
    New-Item -Path $appKey -Force | Out-Null
    Set-ItemProperty -Path $appKey -Name "FriendlyAppName" -Value "Notepad Studio"
    # ApplicationDescription powers the tooltip Explorer shows for the entry.
    Set-ItemProperty -Path $appKey -Name "ApplicationDescription" `
        -Value "Lightweight cross-platform text editor inspired by Notepad++."

    $iconKey = Join-Path $appKey "DefaultIcon"
    New-Item -Path $iconKey -Force | Out-Null
    Set-ItemProperty -Path $iconKey -Name "(default)" -Value "`"$exeDst`",0"

    $cmdKey = Join-Path $appKey "shell\open\command"
    New-Item -Path $cmdKey -Force | Out-Null
    # %1 = the file path Explorer passes when the user picks our entry. Quoted
    # so paths with spaces survive intact.
    Set-ItemProperty -Path $cmdKey -Name "(default)" -Value "`"$exeDst`" `"%1`""

    # SupportedTypes lists every extension we should show up under in "Open
    # With". Each value name is the extension (with leading dot); the value
    # data is intentionally empty — Windows only inspects the value name.
    $typesKey = Join-Path $appKey "SupportedTypes"
    New-Item -Path $typesKey -Force | Out-Null
    foreach ($ext in $extensions) {
        Set-ItemProperty -Path $typesKey -Name $ext -Value ""
    }

    # Also nudge each extension's OpenWithProgids list so Explorer's "Open
    # With" submenu includes us alongside whatever else is registered.
    # Missing extension keys are skipped — Windows materialises them as
    # soon as another app claims the type.
    foreach ($ext in $extensions) {
        $extKey = Join-Path $classesRoot $ext
        if (-not (Test-Path $extKey)) {
            New-Item -Path $extKey -Force | Out-Null
        }
        $owl = Join-Path $extKey "OpenWithProgids"
        New-Item -Path $owl -Force | Out-Null
        Set-ItemProperty -Path $owl -Name "Applications\$exeName" -Value "" -Type None
    }

    Write-Host ""
    Write-Host "Installed:"
    Write-Host "  exe:      $exeDst"
    Write-Host "  registry: $appKey"
    Write-Host ""
    Write-Host "Notepad Studio should now appear in Explorer's Open With submenu"
    Write-Host "for text + source-code files. If you don't see it immediately, run:"
    Write-Host "  ie4uinit.exe -show"
    Write-Host "or log out and back in to flush Explorer's cache."
}

if ($Uninstall) {
    Remove-Install
} else {
    Install-Install
}
