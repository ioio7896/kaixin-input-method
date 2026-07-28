#pragma once

#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>

#include <string>

bool SrfIsSensitiveInputContext(const std::wstring& appName);
