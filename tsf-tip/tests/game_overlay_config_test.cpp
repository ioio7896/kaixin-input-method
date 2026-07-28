#include "ime_config.h"

#include <filesystem>
#include <fstream>

extern "C" void SrfTip_BackgroundWorkerAddRef() {}
extern "C" void SrfTip_BackgroundWorkerRelease() {}

int main() {
  const std::filesystem::path path =
      std::filesystem::temp_directory_path() / L"kaixin-game-overlay-config-test.ini";
  {
    std::ofstream output(path, std::ios::binary | std::ios::trunc);
    if (!output) return 1;
    output << "[app:C:\\Games\\Demo\\game.exe]\n"
              "policy=show_ui\n"
              "game_profile=compact\n"
              "overlay_anchor=top_center\n"
              "overlay_offset_x=-25\n"
              "overlay_offset_y=40\n"
              "overlay_scale=175\n"
              "overlay_monitor=2\n"
              "overlay_backend=external\n";
  }

  const SrfConfig config = LoadSrfConfigFromPath(path);
  std::error_code ignored;
  std::filesystem::remove(path, ignored);
  const SrfAppOptions* options =
      FindAppOptions(config, L"c:\\games\\demo\\GAME.EXE");
  if (!options || !options->hasGameProfile || !options->gameCompactProfile ||
      !options->hasOverlayAnchor || options->overlayAnchor != SrfOverlayAnchor::TopCenter ||
      !options->hasOverlayOffsetX || options->overlayOffsetX != -25 ||
      !options->hasOverlayOffsetY || options->overlayOffsetY != 40 ||
      !options->hasOverlayScale || options->overlayScalePercent != 175 ||
      !options->hasOverlayMonitor || options->overlayMonitor != L"2" ||
      !options->hasOverlayBackend ||
      options->overlayBackend != SrfOverlayBackend::External) {
    return 2;
  }

  const std::filesystem::path clampedPath =
      std::filesystem::temp_directory_path() / L"kaixin-game-overlay-config-clamp-test.ini";
  {
    std::ofstream output(clampedPath, std::ios::binary | std::ios::trunc);
    if (!output) return 3;
    output << "[app:game.exe]\n"
              "overlay_offset_x=-99999\n"
              "overlay_offset_y=99999\n"
              "overlay_scale=999\n"
              "overlay_monitor=99\n"
              "overlay_backend=invalid\n";
  }
  const SrfConfig clamped = LoadSrfConfigFromPath(clampedPath);
  std::filesystem::remove(clampedPath, ignored);
  options = FindAppOptions(clamped, L"game.exe");
  if (!options || options->overlayOffsetX != -4000 || options->overlayOffsetY != 4000 ||
      options->overlayScalePercent != 200 || options->hasOverlayMonitor ||
      options->hasOverlayBackend) {
    return 4;
  }
  return 0;
}
