: << 'CMDBLOCK'
@echo off
REM Cross-platform wrapper for the canon binary.
REM
REM On Windows cmd.exe runs the batch half below. On Unix the shell reads the
REM whole file as a script, where a leading `:` is a no-op and the heredoc
REM swallows the batch lines.
REM
REM The extensionless name is deliberate: Claude Code prepends `bash` to any
REM hook command containing `.sh`, which would break the Windows path.

setlocal
if "%CLAUDE_PLUGIN_ROOT%"=="" set "CLAUDE_PLUGIN_ROOT=%~dp0.."

REM A developer build wins over the installed one, same as the Unix half.
if exist "%CLAUDE_PLUGIN_ROOT%\target\release\canon.exe" (
    "%CLAUDE_PLUGIN_ROOT%\target\release\canon.exe" %*
    exit /b 0
)
if exist "%CLAUDE_PLUGIN_ROOT%\target\debug\canon.exe" (
    "%CLAUDE_PLUGIN_ROOT%\target\debug\canon.exe" %*
    exit /b 0
)

set "CANON_HOME=%CLAUDE_PLUGIN_DATA%"
if "%CANON_HOME%"=="" set "CANON_HOME=%LOCALAPPDATA%\canon"

REM The installed binary carries its version in the name, so that a plugin
REM update cannot leave an older binary answering for a newer plugin. Batch
REM cannot read that version out of Cargo.toml, so it runs whichever one is
REM there; the Unix half deletes the binary it replaces, so there is only ever
REM one.
for %%f in ("%CANON_HOME%\bin\canon-*.exe") do (
    "%%~ff" %*
    exit /b 0
)

REM An unversioned name, for someone who placed a build here by hand.
if exist "%CANON_HOME%\bin\canon.exe" (
    "%CANON_HOME%\bin\canon.exe" %*
    exit /b 0
)

REM Nothing to run. Say nothing, successfully: a hook that reports an error
REM interrupts someone mid-edit over something they cannot act on.
echo {}
exit /b 0
CMDBLOCK

exec "$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)/../bin/canon" "$@"
