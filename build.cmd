@echo off
setlocal
cd /d "%~dp0"
title NZB FF Tool Pro - compilation

if exist "C:\mingw64\bin\gcc.exe" set "PATH=C:\mingw64\bin;%PATH%"

where cargo >nul 2>nul || (echo [ERREUR] cargo introuvable : Rust n'est pas installe. & pause & exit /b 1)
if not exist "C:\mingw64\bin\dlltool.exe" (echo [ERREUR] C:\mingw64\bin\dlltool.exe introuvable : MinGW/WinLibs n'est pas dans C:\mingw64. & pause & exit /b 1)

echo Compilation de NZB FF Tool Pro (la premiere fois : quelques minutes)...
cargo build --release
if errorlevel 1 (echo. & echo [ERREUR] La compilation a echoue : copie les lignes "error" ci-dessus. & pause & exit /b 1)

if not exist dist mkdir dist
copy /y target\release\nzbff.exe "dist\NZB FF Tool Pro.exe" >nul
if exist dist\ngprep.conf if not exist dist\nzbff.conf copy /y dist\ngprep.conf dist\nzbff.conf >nul
if exist dist\ngprep-history.tsv if not exist dist\nzbff-history.tsv copy /y dist\ngprep-history.tsv dist\nzbff-history.tsv >nul
if exist dist\ngprep.exe del dist\ngprep.exe
echo.
echo OK : "dist\NZB FF Tool Pro.exe" est pret (reglages et historique repris).
pause
