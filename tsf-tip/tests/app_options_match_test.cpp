#include "ime_model.h"

namespace {

bool Check(bool condition) { return condition; }

}  // namespace

int main() {
  SrfConfig config = {};

  SrfAppOptions generic = {};
  generic.hasAsciiMode = true;
  generic.asciiMode = true;
  config.appOptions.emplace(L"game.exe", generic);

  SrfAppOptions exact = {};
  exact.hasHideUi = true;
  exact.hideUi = true;
  config.appOptions.emplace(L"C:\\Games\\Special\\game.exe", exact);

  const SrfAppOptions* matched =
      FindAppOptions(config, L"c:\\games\\special\\GAME.EXE");
  if (!Check(matched && matched->hasHideUi && !matched->hasAsciiMode)) return 1;

  matched = FindAppOptions(config, L"C:/Games/Special/game.exe");
  if (!Check(matched && matched->hasHideUi && !matched->hasAsciiMode)) return 7;

  matched = FindAppOptions(config, L"D:\\Games\\Other\\GAME.EXE");
  if (!Check(matched && matched->hasAsciiMode && !matched->hasHideUi)) return 2;

  matched = FindAppOptions(config, L"D:/Games/Other/Game.exe");
  if (!Check(matched && matched->hasAsciiMode)) return 3;

  matched = FindAppOptions(config, L"GAME.EXE");
  if (!Check(matched && matched->hasAsciiMode)) return 4;

  if (!Check(FindAppOptions(config, L"") == nullptr)) return 5;
  if (!Check(FindAppOptions(config, L"C:\\Games\\missing.exe") == nullptr)) return 6;
  return 0;
}
