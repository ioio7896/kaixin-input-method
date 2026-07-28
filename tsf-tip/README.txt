SRF TSF Pinyin TIP (srf_tsf_tip.dll)
====================================

Build (x64 only; CMake will fail on 32-bit generators):
  cmake -B build -G "Visual Studio 18 2026" -A x64
  cmake --build build --config Release

Output:
  build\Release\srf_tsf_tip.dll

Register - per-user (HKCU\Software\Classes, no admin):
  register.cmd
  or: regsvr32 path\to\srf_tsf_tip.dll

Register - all users (HKLM\Software\Classes, requires elevated shell):
  register_machine.cmd
  or: regsvr32 /i:machine path\to\srf_tsf_tip.dll
  (calls DllInstall(TRUE, L"machine"); see dllmain.cpp)

Unregister - user scope (matches default regsvr32):
  unregister.cmd
  or: regsvr32 /u path\to\srf_tsf_tip.dll

Unregister - machine scope:
  unregister_machine.cmd
  or: regsvr32 /u /n /i:machine path\to\srf_tsf_tip.dll

The installer now also adds the input method under Windows language settings
for Chinese (Simplified, China) when that language pack is installed.
Display name: "SRF Pinyin TSF".

Behavior:
- Letters a-z build a pinyin buffer; composition shows reading and candidates.
- Keys 1-9 choose candidates; Space/Enter commit; Esc cancels; Backspace edits.

Notes:
- MSVC: /utf-8 for UTF-8 sources.
- Linking: ole32/oleaut32/uuid/advapi32/user32 (no msctf.lib required for this DLL).
