#pragma once

#include <cstdint>
#include <filesystem>

#include "ime_model.h"

std::filesystem::path ResolveSrfConfigPath();
SrfConfig LoadSrfConfig();
SrfConfig LoadSrfConfigFromPath(const std::filesystem::path& path);
uint64_t GetSrfConfigVersion();
void LoadSkinFile(SrfConfig& config);
