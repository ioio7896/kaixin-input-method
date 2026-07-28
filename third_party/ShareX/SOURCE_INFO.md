# ShareX source provenance

- Upstream project: https://github.com/ShareX/ShareX
- Download URL: https://codeload.github.com/ShareX/ShareX/zip/refs/heads/master
- Upstream commit: `73967140f4fd64ca4b93203ae8ad5ac05ade9aaf`
- Commit date: 2026-07-03
- Upstream version: 21.0.0
- Downloaded: 2026-07-11
- Source ZIP SHA-256: `7024A967C061E153382F60673E6CCAF9AFCE72F5E7C5A2B85627FDDD7AEE9C3F`
- License: GPL-3.0 (see `LICENSE.txt`)

## Local integration changes

The vendored source is modified for the Kaixin input method integration:

- Adds private `-KaixinRectangleRegion`, `-KaixinCaptureWindow <HWND>`, and `-KaixinOutputPath <path>` commands.
- Those commands always open the ShareX image editor, save the accepted result to the exact optional path supplied by Kaixin, and copy it to the clipboard.
- Clipboard handoff publishes ShareX's lossless PNG format so Kaixin does not depend on GDI bitmap channel/alpha conversion.
- Upload tasks remain disabled; automatic file saving is restricted to the local path explicitly supplied by the Kaixin tray process.
- Uses an integration-specific mutex/pipe so an independently installed ShareX instance is not affected.
- Reuses that private pipe for warm consecutive captures and writes a per-request completion result so cancellation immediately releases the input method's capture lock.
- Suppresses the ShareX tray icon and global hotkeys while running in integration mode.
- Uses the process name `KaixinShareX.exe` so installation maintenance never terminates a separately installed ShareX instance.
- Publishes the bundled integration as a compressed self-contained single-file app; standalone image-editor and browser native-messaging host executables are not distributed because the Kaixin capture flow uses the in-process editor and has browser integration disabled.

The complete corresponding source is retained in this directory and is packaged with binary distributions.

## Reproducible build command

From the Kaixin IME repository root, with the .NET 9 SDK installed:

```powershell
$env:AVALONIA_TELEMETRY_OPTOUT = "1"
dotnet publish third_party/ShareX/ShareX/ShareX.csproj `
  -c Release -r win-x64 --self-contained true --nologo `
  -p:DebugType=None -p:DebugSymbols=false `
  -p:PublishSingleFile=true -p:EnableCompressionInSingleFile=true `
  -p:IncludeNativeLibrariesForSelfExtract=false `
  -o third_party/ShareX/publish/win-x64
```

`python build.py` runs the same command, removes development-only symbol/import files, checks the
Kaixin integration changes, and creates `dist/kaixin-sharex-corresponding-source.zip`. The source
archive must be published beside every installer that contains `KaixinShareX.exe`.
