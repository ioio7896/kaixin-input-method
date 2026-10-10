#include <windows.h>
#include <cstdlib>
#include <iostream>
#include <string>
#include "ime_config.h"

extern "C" void SrfTip_BackgroundWorkerAddRef() {}
extern "C" void SrfTip_BackgroundWorkerRelease() {}

int main() {
  wchar_t directory[MAX_PATH] = {}, file[MAX_PATH] = {};
  if (!GetTempPathW(MAX_PATH, directory) || !GetTempFileNameW(directory, L"kxg", 0, file)) return 1;
  const char* modes[] = {"manual", "passthrough", "chinese", "auto_text"};
  const SrfGameInputMode expected[] = {SrfGameInputMode::Manual, SrfGameInputMode::Passthrough,
                                      SrfGameInputMode::Chinese, SrfGameInputMode::AutoText};
  bool ok = true;
  for (int i = 0; i < 4; ++i) {
    const std::string ini = std::string("[compatibility]\r\ngame_input_mode=") + modes[i] +
        "\r\n[input]\r\nchinese_halfwidth=" + (i % 2 ? "1" : "0") + "\r\n[app:mygame.exe]\r\ngame_profile=compact\r\ngame_input_mode=" + modes[i] +
        "\r\ncommit_transport=clipboard_paste\r\noverlay_anchor=bottom_left\r\n"
        "game_enter_behavior=stay\r\ngame_auto_uia=0\r\ngame_status_indicator=0\r\n"
        "game_chat_open_key=Enter\r\ngame_chat_close_key=Escape\r\noverlay_force_ui=1\r\n";
    HANDLE handle = CreateFileW(file, GENERIC_WRITE, 0, nullptr, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, nullptr);
    DWORD written = 0;
    if (handle == INVALID_HANDLE_VALUE) { ok = false; break; }
    const bool saved = WriteFile(handle, ini.data(), static_cast<DWORD>(ini.size()), &written, nullptr) && written == ini.size();
    CloseHandle(handle);
    if (!saved) { ok = false; break; }
    const auto config = LoadSrfConfigFromPath(file);
    const auto* app = FindAppOptions(config, L"mygame.exe");
    ok = ok && config.input.chineseHalfwidth == (i % 2 != 0) && config.compatibility.gameInputMode == expected[i] && app && app->hasGameInputMode &&
        app->gameInputMode == expected[i] && app->hasCommitTransport &&
        app->commitTransport == SrfCommitTransport::ClipboardPaste &&
        app->overlayAnchor == SrfOverlayAnchor::BottomLeft && config.input.gameModeHotkey.enabled;
    ok = ok && app && app->hasGameEnterBehavior && app->gameEnterBehavior == SrfGameEnterBehavior::Stay &&
        app->hasGameAutoUia && !app->gameAutoUia && app->hasGameStatusIndicator && !app->gameStatusIndicator &&
        app->gameChatOpenKey.enabled && app->gameChatOpenKey.vk == VK_RETURN && app->gameChatOpenKey.modifiers == 0 &&
        app->gameChatCloseKey.enabled && app->gameChatCloseKey.vk == VK_ESCAPE && app->overlayForceUi &&
        config.compatibility.gameAutoUia && config.compatibility.gameStatusIndicator &&
        config.compatibility.gameEnterBehavior == SrfGameEnterBehavior::Auto;
  }
  DeleteFileW(file);
  if (!ok) { std::cerr << "Game configuration test failed\n"; return 1; }
  std::cout << "Game configuration tests passed\n";
  return 0;
}
