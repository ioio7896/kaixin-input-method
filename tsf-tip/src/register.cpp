#include <msctf.h>
#include <objbase.h>
#include <windows.h>

#include <cstdio>
#include <filesystem>
#include <string>

#include "guids.h"

#pragma comment(lib, "advapi32.lib")
#pragma comment(lib, "ole32.lib")

namespace {

std::wstring ClsidToString(REFGUID guid) {
  WCHAR buf[64];
  if (StringFromGUID2(guid, buf, 64) == 0) return {};
  return buf;
}

HRESULT RegSetSz(HKEY hKey, const WCHAR* valueName, const WCHAR* data) {
  const SIZE_T cb = (wcslen(data) + 1) * sizeof(WCHAR);
  return HRESULT_FROM_WIN32(RegSetValueExW(hKey, valueName, 0, REG_SZ,
                                           reinterpret_cast<const BYTE*>(data),
                                           static_cast<DWORD>(cb)));
}

HKEY ClassesRootKey(BOOL machineScope) { return machineScope ? HKEY_LOCAL_MACHINE : HKEY_CURRENT_USER; }

void AppendRegisterLog(const wchar_t* message) {
  DWORD len = GetEnvironmentVariableW(L"SRF_TSF_REG_LOG", nullptr, 0);
  if (len == 0) return;

  std::wstring path(len, L'\0');
  if (GetEnvironmentVariableW(L"SRF_TSF_REG_LOG", path.data(), len) == 0) return;
  if (!path.empty() && path.back() == L'\0') path.pop_back();
  if (path.empty()) return;

  HANDLE file = CreateFileW(path.c_str(), FILE_APPEND_DATA, FILE_SHARE_READ, nullptr, OPEN_ALWAYS,
                            FILE_ATTRIBUTE_NORMAL, nullptr);
  if (file == INVALID_HANDLE_VALUE) return;

  SYSTEMTIME st = {};
  GetLocalTime(&st);
  wchar_t line[1024];
  const int written = _snwprintf_s(line, _countof(line), _TRUNCATE, L"[%04u-%02u-%02u %02u:%02u:%02u] %s\r\n",
                                   st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond,
                                   message ? message : L"");
  if (written > 0) {
    DWORD bytes = 0;
    WriteFile(file, line, static_cast<DWORD>(written * sizeof(wchar_t)), &bytes, nullptr);
  }
  CloseHandle(file);
}

void AppendRegisterLogHr(const wchar_t* step, HRESULT hr) {
  wchar_t line[512];
  _snwprintf_s(line, _countof(line), _TRUNCATE, L"%s hr=0x%08X", step ? step : L"(null)",
               static_cast<unsigned int>(hr));
  AppendRegisterLog(line);
}

void AppendRegisterLogDword(const wchar_t* step, DWORD value) {
  wchar_t line[512];
  _snwprintf_s(line, _countof(line), _TRUNCATE, L"%s value=0x%08X", step ? step : L"(null)",
               static_cast<unsigned int>(value));
  AppendRegisterLog(line);
}

HRESULT RegisterDefaultIcon(BOOL machineScope, const std::wstring& iconFilePath) {
  const std::wstring clsidStr = ClsidToString(CLSID_SrfTsfTip);
  if (clsidStr.empty() || iconFilePath.empty()) return E_FAIL;

  const std::wstring keyPath = L"Software\\Classes\\CLSID\\" + clsidStr + L"\\DefaultIcon";
  HKEY hKey = nullptr;
  const LONG lr = RegCreateKeyExW(ClassesRootKey(machineScope), keyPath.c_str(), 0, nullptr, 0,
                                  KEY_WRITE, nullptr, &hKey, nullptr);
  AppendRegisterLogHr(L"RegCreateKeyExW(DefaultIcon)", HRESULT_FROM_WIN32(lr));
  if (lr != ERROR_SUCCESS) return HRESULT_FROM_WIN32(lr);

  const std::wstring value = iconFilePath + L",0";
  HRESULT hr = RegSetSz(hKey, nullptr, value.c_str());
  AppendRegisterLogHr(L"RegSetSz(DefaultIcon)", hr);
  RegCloseKey(hKey);
  return hr;
}

std::wstring BuildTipIdentifier(LANGID lang) {
  const std::wstring clsid = ClsidToString(CLSID_SrfTsfTip);
  const std::wstring profile = ClsidToString(GUID_PROFILE_SRF);
  if (clsid.empty() || profile.empty()) return {};

  wchar_t prefix[16];
  _snwprintf_s(prefix, _countof(prefix), _TRUNCATE, L"%04X:", static_cast<unsigned int>(lang));
  return std::wstring(prefix) + clsid + profile;
}

HRESULT EnableProfileForCurrentUser(LANGID lang) {
  const std::wstring tipId = BuildTipIdentifier(lang);
  if (tipId.empty()) return E_FAIL;

  AppendRegisterLog(L"EnableProfileForCurrentUser begin");
  AppendRegisterLog(tipId.c_str());

  HRESULT bestHr = E_FAIL;
  HMODULE inputDll = LoadLibraryExW(L"input.dll", nullptr, LOAD_LIBRARY_SEARCH_SYSTEM32);
  if (inputDll != nullptr) {
    using QueryLayoutOrTipStringFn = DWORD(WINAPI*)(LPCWSTR, DWORD);
    using InstallLayoutOrTipFn = BOOL(WINAPI*)(LPCWSTR, DWORD);

    auto queryFn = reinterpret_cast<QueryLayoutOrTipStringFn>(
        GetProcAddress(inputDll, "QueryLayoutOrTipString"));
    auto installFn =
        reinterpret_cast<InstallLayoutOrTipFn>(GetProcAddress(inputDll, "InstallLayoutOrTip"));

    if (queryFn != nullptr) {
      const DWORD queryResult = queryFn(tipId.c_str(), 0);
      AppendRegisterLogDword(L"QueryLayoutOrTipString", queryResult);
      bestHr = (queryResult == ERROR_SUCCESS) ? S_OK : HRESULT_FROM_WIN32(queryResult);
    } else {
      AppendRegisterLog(L"QueryLayoutOrTipString export not found");
    }

    if (installFn != nullptr) {
      SetLastError(ERROR_SUCCESS);
      if (installFn(tipId.c_str(), 0)) {
        AppendRegisterLog(L"InstallLayoutOrTip succeeded");
        FreeLibrary(inputDll);
        return S_OK;
      }

      const DWORD err = GetLastError();
      bestHr = HRESULT_FROM_WIN32(err == ERROR_SUCCESS ? ERROR_GEN_FAILURE : err);
      AppendRegisterLogHr(L"InstallLayoutOrTip", bestHr);
    } else {
      AppendRegisterLog(L"InstallLayoutOrTip export not found");
    }

    FreeLibrary(inputDll);
  } else {
    bestHr = HRESULT_FROM_WIN32(GetLastError());
    AppendRegisterLogHr(L"LoadLibraryExW(input.dll, SYSTEM32)", bestHr);
  }

  ITfInputProcessorProfileMgr* profileMgr = nullptr;
  HRESULT hr = CoCreateInstance(CLSID_TF_InputProcessorProfiles, nullptr, CLSCTX_INPROC_SERVER,
                                IID_ITfInputProcessorProfileMgr,
                                reinterpret_cast<void**>(&profileMgr));
  AppendRegisterLogHr(L"CoCreateInstance(IID_ITfInputProcessorProfileMgr)", hr);
  if (SUCCEEDED(hr) && profileMgr != nullptr) {
    hr = profileMgr->ActivateProfile(TF_PROFILETYPE_INPUTPROCESSOR, lang, CLSID_SrfTsfTip,
                                     GUID_PROFILE_SRF, nullptr,
                                     TF_IPPMF_ENABLEPROFILE | TF_IPPMF_FORSESSION |
                                         TF_IPPMF_DONTCARECURRENTINPUTLANGUAGE);
    AppendRegisterLogHr(L"ITfInputProcessorProfileMgr::ActivateProfile", hr);
    profileMgr->Release();
    if (SUCCEEDED(hr) || hr == TF_E_ALREADY_EXISTS) return S_OK;
    if (FAILED(bestHr)) return bestHr;
    return hr;
  }

  return FAILED(bestHr) ? bestHr : hr;
}

HRESULT RegisterCategories() {
  ITfCategoryMgr* categoryMgr = nullptr;
  HRESULT hr = CoCreateInstance(CLSID_TF_CategoryMgr, nullptr, CLSCTX_INPROC_SERVER,
                                IID_ITfCategoryMgr, reinterpret_cast<void**>(&categoryMgr));
  AppendRegisterLogHr(L"RegisterCategories/CoCreateInstance(CLSID_TF_CategoryMgr)", hr);
  if (FAILED(hr)) return hr;

  // 某些系统环境下（尤其是仅用户范围注册、或系统策略限制写入 TSF 类别存储）RegisterCategory
  // 可能返回 E_FAIL，但 TIP 的注册与启用仍可通过 ProfileMgr/Profiles 完成。
  // 因此这里对 RegisterCategory 采取“尽力而为”策略：记录日志但不阻断安装。
  HRESULT hrCat = categoryMgr->RegisterCategory(CLSID_SrfTsfTip, GUID_TFCAT_TIP_KEYBOARD, CLSID_SrfTsfTip);
  AppendRegisterLogHr(L"RegisterCategories/RegisterCategory TIP_KEYBOARD", hrCat);

  hrCat = categoryMgr->RegisterCategory(CLSID_SrfTsfTip, GUID_TFCAT_TIPCAP_SYSTRAYSUPPORT,
                                        CLSID_SrfTsfTip);
  AppendRegisterLogHr(L"RegisterCategories/RegisterCategory TIPCAP_SYSTRAYSUPPORT", hrCat);

  hrCat = categoryMgr->RegisterCategory(CLSID_SrfTsfTip, GUID_TFCAT_TIPCAP_IMMERSIVESUPPORT,
                                        CLSID_SrfTsfTip);
  AppendRegisterLogHr(L"RegisterCategories/RegisterCategory TIPCAP_IMMERSIVESUPPORT", hrCat);

  hrCat = categoryMgr->RegisterCategory(CLSID_SrfTsfTip, GUID_TFCAT_TIPCAP_UIELEMENTENABLED,
                                        CLSID_SrfTsfTip);
  AppendRegisterLogHr(L"RegisterCategories/RegisterCategory TIPCAP_UIELEMENTENABLED", hrCat);

  hrCat = categoryMgr->RegisterCategory(CLSID_SrfTsfTip, GUID_TFCAT_TIPCAP_SECUREMODE,
                                        CLSID_SrfTsfTip);
  AppendRegisterLogHr(L"RegisterCategories/RegisterCategory TIPCAP_SECUREMODE", hrCat);

  hrCat = categoryMgr->RegisterCategory(CLSID_SrfTsfTip, GUID_TFCAT_DISPLAYATTRIBUTEPROVIDER, CLSID_SrfTsfTip);
  AppendRegisterLogHr(L"RegisterCategories/RegisterCategory DISPLAYATTRIBUTEPROVIDER(0)", hrCat);

  hrCat = categoryMgr->RegisterCategory(CLSID_SrfTsfTip, GUID_TFCAT_DISPLAYATTRIBUTEPROVIDER, GUID_DISPLAY_ATTRIBUTE_SRF_INPUT);
  AppendRegisterLogHr(L"RegisterCategories/RegisterCategory DISPLAYATTRIBUTEPROVIDER(SRF_INPUT)", hrCat);

  hrCat = categoryMgr->RegisterCategory(CLSID_SrfTsfTip, GUID_TFCAT_TIPCAP_INPUTMODECOMPARTMENT, CLSID_SrfTsfTip);
  AppendRegisterLogHr(L"RegisterCategories/RegisterCategory TIPCAP_INPUTMODECOMPARTMENT", hrCat);
  categoryMgr->Release();
  return S_OK;
}

HRESULT RegisterProfiles(const WCHAR* dllPath, BOOL machineScope) {
  const LANGID lang = MAKELANGID(LANG_CHINESE, SUBLANG_CHINESE_SIMPLIFIED);
  const WCHAR* name = L"\u5f00\u5fc3\u8f93\u5165\u6cd5";
  const DWORD registerFlags = 0;
  HRESULT hr = E_FAIL;
  bool registered = false;

  // RegisterProfile 的 pchIconFile 参数要求“包含图标资源的文件”；
  // 直接用 TIP 的 DLL 作为 icon source 很可能因为缺少图标资源而导致 E_FAIL。
  // 优先选择安装目录下更可能带图标的文件作为 icon file。
  std::wstring iconFilePath = dllPath;
  try {
    const std::filesystem::path tipDllPath(dllPath);
    const std::filesystem::path baseDir = tipDllPath.parent_path();

    bool selectedIcon = false;
    std::filesystem::path probeDir = baseDir;
    for (int depth = 0; depth < 5 && !probeDir.empty() && !selectedIcon; ++depth) {
      const std::filesystem::path iconCandidates[] = {
          probeDir / L"assets" / L"kaixin-input.ico",
          probeDir / L"icons" / L"app_icon.ico",
          probeDir / L"kaixin-input.ico",
          probeDir / L"app_icon.ico",
      };
      for (const auto& iconCandidate : iconCandidates) {
        if (std::filesystem::exists(iconCandidate)) {
          iconFilePath = iconCandidate.wstring();
          selectedIcon = true;
          break;
        }
      }
      if (selectedIcon) break;

      const std::filesystem::path settingsExe = probeDir / L"srf_ime_settings.exe";
      if (std::filesystem::exists(settingsExe)) {
        iconFilePath = settingsExe.wstring();
        selectedIcon = true;
        break;
      }

      const std::filesystem::path trayExe = probeDir / L"srf_ime_tray.exe";
      if (std::filesystem::exists(trayExe)) {
        iconFilePath = trayExe.wstring();
        selectedIcon = true;
        break;
      }

      const std::filesystem::path parent = probeDir.parent_path();
      if (parent == probeDir) {
        break;
      }
      probeDir = parent;
    }

    if (!selectedIcon) {
      const std::filesystem::path engineExe = baseDir / L"srf_ime_engine.exe";
      if (std::filesystem::exists(engineExe)) {
        iconFilePath = engineExe.wstring();
      }
    }
  } catch (...) {
    // ignore: keep dllPath as fallback
  }

  const ULONG iconFileLen = static_cast<ULONG>(wcslen(iconFilePath.c_str()));
  const HKL hklSubstitute = GetKeyboardLayout(0);

  AppendRegisterLog(L"RegisterProfiles begin");
  AppendRegisterLog(L"RegisterProfiles selected iconFilePath");
  AppendRegisterLog(iconFilePath.c_str());
  AppendRegisterLogHr(L"RegisterDefaultIcon", RegisterDefaultIcon(machineScope, iconFilePath));
  {
    wchar_t hklBuf[64] = {};
    _snwprintf_s(hklBuf, _countof(hklBuf), _TRUNCATE, L"HKL=0x%p", hklSubstitute);
    AppendRegisterLog(hklBuf);
  }

  ITfInputProcessorProfileMgr* profileMgr = nullptr;
  hr = CoCreateInstance(CLSID_TF_InputProcessorProfiles, nullptr, CLSCTX_INPROC_SERVER,
                        IID_ITfInputProcessorProfileMgr, reinterpret_cast<void**>(&profileMgr));
  AppendRegisterLogHr(L"CoCreateInstance(IID_ITfInputProcessorProfileMgr)", hr);
  if (SUCCEEDED(hr) && profileMgr) {
    hr = profileMgr->RegisterProfile(CLSID_SrfTsfTip, lang, GUID_PROFILE_SRF, name,
                                     static_cast<ULONG>(wcslen(name)),
                                     iconFilePath.c_str(), iconFileLen, 0, hklSubstitute, 0, TRUE,
                                     registerFlags);
    AppendRegisterLogHr(L"ITfInputProcessorProfileMgr::RegisterProfile", hr);
    profileMgr->Release();
    if (SUCCEEDED(hr) || hr == TF_E_ALREADY_EXISTS) registered = true;
  }

  if (!registered) {
    ITfInputProcessorProfiles* profiles = nullptr;
    hr = CoCreateInstance(CLSID_TF_InputProcessorProfiles, nullptr, CLSCTX_INPROC_SERVER,
                          IID_ITfInputProcessorProfiles, reinterpret_cast<void**>(&profiles));
    AppendRegisterLogHr(L"CoCreateInstance(CLSID_TF_InputProcessorProfiles)", hr);
    if (FAILED(hr)) return hr;

    hr = profiles->Register(CLSID_SrfTsfTip);
    AppendRegisterLogHr(L"ITfInputProcessorProfiles::Register", hr);
    if (FAILED(hr) && hr != TF_E_ALREADY_EXISTS) {
      if (!machineScope && hr == E_FAIL) {
        AppendRegisterLog(
            L"ITfInputProcessorProfiles::Register returned E_FAIL in user scope; "
            L"continuing because the profile may already exist at machine scope");
        registered = true;
      } else {
        profiles->Release();
        return hr;
      }
    } else {
      hr = profiles->AddLanguageProfile(CLSID_SrfTsfTip, lang, GUID_PROFILE_SRF, name,
                                        static_cast<ULONG>(wcslen(name)),
                                        iconFilePath.c_str(), iconFileLen, 0);
      AppendRegisterLogHr(L"ITfInputProcessorProfiles::AddLanguageProfile", hr);
      if (FAILED(hr) && hr != TF_E_ALREADY_EXISTS) {
        if (!machineScope && hr == E_FAIL) {
          AppendRegisterLog(
              L"ITfInputProcessorProfiles::AddLanguageProfile returned E_FAIL in user scope; "
              L"continuing because the language profile may already exist at machine scope");
          registered = true;
        } else {
          profiles->Release();
          return hr;
        }
      } else {
        registered = true;
      }
    }

    profiles->Release();
  }

  ITfInputProcessorProfiles* profiles = nullptr;
  hr = CoCreateInstance(CLSID_TF_InputProcessorProfiles, nullptr, CLSCTX_INPROC_SERVER,
                        IID_ITfInputProcessorProfiles, reinterpret_cast<void**>(&profiles));
  AppendRegisterLogHr(L"CoCreateInstance(CLSID_TF_InputProcessorProfiles)/Enable", hr);
  if (FAILED(hr)) return hr;

  hr = profiles->EnableLanguageProfile(CLSID_SrfTsfTip, lang, GUID_PROFILE_SRF, TRUE);
  AppendRegisterLogHr(L"ITfInputProcessorProfiles::EnableLanguageProfile", hr);
  if (FAILED(hr) && hr != TF_E_ALREADY_EXISTS) {
    profiles->Release();
    return hr;
  }

  HRESULT defaultHr =
      profiles->EnableLanguageProfileByDefault(CLSID_SrfTsfTip, lang, GUID_PROFILE_SRF, TRUE);
  AppendRegisterLogHr(L"ITfInputProcessorProfiles::EnableLanguageProfileByDefault", defaultHr);
  profiles->Release();
  if (FAILED(defaultHr) && defaultHr != TF_E_ALREADY_EXISTS) {
    if (!machineScope && defaultHr == E_FAIL) {
      AppendRegisterLog(
          L"ITfInputProcessorProfiles::EnableLanguageProfileByDefault returned E_FAIL in user "
          L"scope; continuing because the machine-scope profile may already be the default");
    } else {
      return defaultHr;
    }
  }

  if (machineScope) {
    AppendRegisterLog(L"RegisterProfiles machine scope requested");
  }

  hr = EnableProfileForCurrentUser(lang);
  AppendRegisterLogHr(L"EnableProfileForCurrentUser", hr);
  if (FAILED(hr)) return hr;

  return S_OK;
}

void UnregisterProfiles(BOOL machineScope) {
  (void)machineScope;
  const LANGID lang = MAKELANGID(LANG_CHINESE, SUBLANG_CHINESE_SIMPLIFIED);
  const DWORD profileFlags = 0;

  ITfInputProcessorProfileMgr* profileMgr = nullptr;
  HRESULT hr = CoCreateInstance(CLSID_TF_InputProcessorProfiles, nullptr, CLSCTX_INPROC_SERVER,
                                IID_ITfInputProcessorProfileMgr,
                                reinterpret_cast<void**>(&profileMgr));
  AppendRegisterLogHr(L"CoCreateInstance(IID_ITfInputProcessorProfileMgr)", hr);
  if (SUCCEEDED(hr) && profileMgr) {
    hr = profileMgr->UnregisterProfile(CLSID_SrfTsfTip, lang, GUID_PROFILE_SRF, profileFlags);
    AppendRegisterLogHr(L"ITfInputProcessorProfileMgr::UnregisterProfile", hr);
    profileMgr->Release();
  }

  ITfInputProcessorProfiles* profiles = nullptr;
  hr = CoCreateInstance(CLSID_TF_InputProcessorProfiles, nullptr, CLSCTX_INPROC_SERVER,
                        IID_ITfInputProcessorProfiles, reinterpret_cast<void**>(&profiles));
  AppendRegisterLogHr(L"CoCreateInstance(CLSID_TF_InputProcessorProfiles)", hr);
  if (SUCCEEDED(hr) && profiles) {
    (void)profiles->EnableLanguageProfile(CLSID_SrfTsfTip, lang, GUID_PROFILE_SRF, FALSE);
    (void)profiles->RemoveLanguageProfile(CLSID_SrfTsfTip, lang, GUID_PROFILE_SRF);
    (void)profiles->Unregister(CLSID_SrfTsfTip);
    profiles->Release();
  }
}

void UnregisterCategories() {
  ITfCategoryMgr* categoryMgr = nullptr;
  if (FAILED(CoCreateInstance(CLSID_TF_CategoryMgr, nullptr, CLSCTX_INPROC_SERVER,
                              IID_ITfCategoryMgr, reinterpret_cast<void**>(&categoryMgr)))) {
    return;
  }

  (void)categoryMgr->UnregisterCategory(CLSID_SrfTsfTip, GUID_TFCAT_TIP_KEYBOARD, CLSID_SrfTsfTip);
  (void)categoryMgr->UnregisterCategory(CLSID_SrfTsfTip, GUID_TFCAT_TIPCAP_SYSTRAYSUPPORT,
                                        CLSID_SrfTsfTip);
  (void)categoryMgr->UnregisterCategory(CLSID_SrfTsfTip, GUID_TFCAT_TIPCAP_IMMERSIVESUPPORT,
                                        CLSID_SrfTsfTip);
  (void)categoryMgr->UnregisterCategory(CLSID_SrfTsfTip, GUID_TFCAT_TIPCAP_UIELEMENTENABLED,
                                        CLSID_SrfTsfTip);
  (void)categoryMgr->UnregisterCategory(CLSID_SrfTsfTip, GUID_TFCAT_TIPCAP_SECUREMODE,
                                        CLSID_SrfTsfTip);
  (void)categoryMgr->UnregisterCategory(CLSID_SrfTsfTip, GUID_TFCAT_DISPLAYATTRIBUTEPROVIDER,
                                        CLSID_SrfTsfTip);
  (void)categoryMgr->UnregisterCategory(CLSID_SrfTsfTip, GUID_TFCAT_DISPLAYATTRIBUTEPROVIDER,
                                        GUID_DISPLAY_ATTRIBUTE_SRF_INPUT);
  (void)categoryMgr->UnregisterCategory(CLSID_SrfTsfTip, GUID_TFCAT_TIPCAP_INPUTMODECOMPARTMENT,
                                        CLSID_SrfTsfTip);
  categoryMgr->Release();
}

}  // namespace

HRESULT SrfTip_RegisterServerEx(HMODULE hModule, BOOL machineScope) {
  WCHAR szPath[MAX_PATH];
  AppendRegisterLog(machineScope ? L"SrfTip_RegisterServerEx begin machine" : L"SrfTip_RegisterServerEx begin user");
  if (!GetModuleFileNameW(hModule, szPath, MAX_PATH)) return HRESULT_FROM_WIN32(GetLastError());
  AppendRegisterLog(szPath);

  const std::wstring clsidStr = ClsidToString(CLSID_SrfTsfTip);
  if (clsidStr.empty()) return E_FAIL;

  const std::wstring keyPath = L"Software\\Classes\\CLSID\\" + clsidStr;
  HKEY hRoot = ClassesRootKey(machineScope);

  HKEY hKey = nullptr;
  LONG lr = RegCreateKeyExW(hRoot, keyPath.c_str(), 0, nullptr, 0, KEY_WRITE, nullptr, &hKey, nullptr);
  AppendRegisterLogHr(L"RegCreateKeyExW(CLSID)", HRESULT_FROM_WIN32(lr));
  if (lr != ERROR_SUCCESS) return HRESULT_FROM_WIN32(lr);

  HRESULT hr = RegSetSz(hKey, nullptr, L"\u5f00\u5fc3\u8f93\u5165\u6cd5\u6587\u672c\u8f93\u5165\u5904\u7406\u5668");
  AppendRegisterLogHr(L"RegSetSz(CLSID default)", hr);
  RegCloseKey(hKey);
  if (FAILED(hr)) return hr;

  const std::wstring inproc = keyPath + L"\\InprocServer32";
  lr = RegCreateKeyExW(hRoot, inproc.c_str(), 0, nullptr, 0, KEY_WRITE, nullptr, &hKey, nullptr);
  AppendRegisterLogHr(L"RegCreateKeyExW(InprocServer32)", HRESULT_FROM_WIN32(lr));
  if (lr != ERROR_SUCCESS) return HRESULT_FROM_WIN32(lr);

  hr = RegSetSz(hKey, nullptr, szPath);
  AppendRegisterLogHr(L"RegSetSz(Inproc default)", hr);
  if (SUCCEEDED(hr)) hr = RegSetSz(hKey, L"ThreadingModel", L"Apartment");
  AppendRegisterLogHr(L"RegSetSz(ThreadingModel)", hr);
  RegCloseKey(hKey);
  if (FAILED(hr)) return hr;

  HRESULT hrCom = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
  AppendRegisterLogHr(L"CoInitializeEx", hrCom);
  if (FAILED(hrCom) && hrCom != RPC_E_CHANGED_MODE) return hrCom;
  const bool shouldUninit = SUCCEEDED(hrCom);

  hr = RegisterCategories();
  AppendRegisterLogHr(L"RegisterCategories", hr);
  if (FAILED(hr)) {
    if (shouldUninit) CoUninitialize();
    AppendRegisterLogHr(L"SrfTip_RegisterServerEx end", hr);
    return hr;
  }

  hr = RegisterProfiles(szPath, machineScope);
  AppendRegisterLogHr(L"RegisterProfiles", hr);
  if (shouldUninit) CoUninitialize();
  AppendRegisterLogHr(L"SrfTip_RegisterServerEx end", hr);
  return hr;
}

HRESULT SrfTip_UnregisterServerEx(BOOL machineScope) {
  HRESULT hrCom = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
  AppendRegisterLog(machineScope ? L"SrfTip_UnregisterServerEx begin machine" : L"SrfTip_UnregisterServerEx begin user");
  AppendRegisterLogHr(L"CoInitializeEx", hrCom);
  if (FAILED(hrCom) && hrCom != RPC_E_CHANGED_MODE) return hrCom;
  const bool shouldUninit = SUCCEEDED(hrCom);

  UnregisterProfiles(machineScope);

  UnregisterCategories();
  if (shouldUninit) CoUninitialize();

  const std::wstring clsidStr = ClsidToString(CLSID_SrfTsfTip);
  if (!clsidStr.empty()) {
    const std::wstring keyPath = L"Software\\Classes\\CLSID\\" + clsidStr;
    RegDeleteTreeW(ClassesRootKey(machineScope), keyPath.c_str());
  }

  return S_OK;
}

HRESULT SrfTip_RegisterServer(HMODULE hModule) { return SrfTip_RegisterServerEx(hModule, FALSE); }

HRESULT SrfTip_UnregisterServer() { return SrfTip_UnregisterServerEx(FALSE); }
