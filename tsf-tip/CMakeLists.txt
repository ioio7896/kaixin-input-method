cmake_minimum_required(VERSION 3.20)
if(POLICY CMP0091)
  cmake_policy(SET CMP0091 NEW)
endif()
project(srf_tsf_tip LANGUAGES CXX)

if(MSVC)
  # Apply exploit mitigations consistently to the TIP, helpers and tests.
  add_compile_options(/GS /sdl /guard:cf)
  add_link_options(/DYNAMICBASE /NXCOMPAT /guard:cf /CETCOMPAT)
  if(CMAKE_SIZEOF_VOID_P EQUAL 8)
    add_link_options(/HIGHENTROPYVA)
  endif()
endif()

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

set(SRF_APP_VERSION "dev")
set(SRF_VERSION_FILE "${CMAKE_CURRENT_SOURCE_DIR}/../VERSION")
set_property(DIRECTORY APPEND PROPERTY CMAKE_CONFIGURE_DEPENDS "${SRF_VERSION_FILE}")
if(EXISTS "${SRF_VERSION_FILE}")
  file(READ "${SRF_VERSION_FILE}" SRF_APP_VERSION_FILE)
  string(STRIP "${SRF_APP_VERSION_FILE}" SRF_APP_VERSION)
endif()

set(SRF_GIT_COMMIT "unknown")
execute_process(
  COMMAND git rev-parse --short=12 HEAD
  WORKING_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}/.."
  OUTPUT_VARIABLE SRF_GIT_COMMIT_OUT
  ERROR_QUIET
  OUTPUT_STRIP_TRAILING_WHITESPACE
)
if(SRF_GIT_COMMIT_OUT)
  set(SRF_GIT_COMMIT "${SRF_GIT_COMMIT_OUT}")
endif()

set(SRF_GIT_DIRTY 0)
execute_process(
  COMMAND git status --porcelain
  WORKING_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}/.."
  OUTPUT_VARIABLE SRF_GIT_STATUS_OUT
  ERROR_QUIET
  OUTPUT_STRIP_TRAILING_WHITESPACE
)
if(SRF_GIT_STATUS_OUT)
  set(SRF_GIT_DIRTY 1)
endif()

# Re-run configure when HEAD moves so SRF_GIT_COMMIT is re-stamped even when
# no C++ source changed (same pattern as the VERSION CONFIGURE_DEPENDS above).
execute_process(
  COMMAND git rev-parse --absolute-git-dir
  WORKING_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}/.."
  OUTPUT_VARIABLE SRF_GIT_DIR_ABS
  ERROR_QUIET
  OUTPUT_STRIP_TRAILING_WHITESPACE
)
if(SRF_GIT_DIR_ABS AND EXISTS "${SRF_GIT_DIR_ABS}/HEAD")
  set_property(DIRECTORY APPEND PROPERTY CMAKE_CONFIGURE_DEPENDS
    "${SRF_GIT_DIR_ABS}/HEAD")
endif()

add_library(srf_tsf_tip SHARED
  src/dllmain.cpp
  src/factory.cpp
  src/guids.cpp
  src/candidate_overlay_client.cpp
  src/candidate_overlay_placement.cpp
  src/candidate_ui.cpp
  src/candidate_window.cpp
  src/composition_sink.cpp
  src/ime_config.cpp
  src/pinyin_stub.cpp
  src/privacy_sensitive_context.cpp
  src/srf_tip.cpp
  src/key_sink.cpp
  src/notification_window.cpp
  src/thread_mgr_sink.cpp
  src/register.cpp
)

target_include_directories(srf_tsf_tip PRIVATE "${CMAKE_CURRENT_SOURCE_DIR}/include")

target_compile_definitions(srf_tsf_tip PRIVATE
  UNICODE
  _UNICODE
  SRF_APP_VERSION="${SRF_APP_VERSION}"
  SRF_GIT_COMMIT="${SRF_GIT_COMMIT}"
  SRF_GIT_DIRTY=${SRF_GIT_DIRTY}
)

if(MSVC)
  set_property(TARGET srf_tsf_tip PROPERTY
    MSVC_RUNTIME_LIBRARY "MultiThreaded$<$<CONFIG:Debug>:Debug>")
endif()

target_link_libraries(srf_tsf_tip PRIVATE
  ole32
  oleaut32
  uuid
  advapi32
  crypt32
  d2d1
  dwmapi
  dwrite
  user32
  shell32
  gdi32
  gdiplus
  msimg32
)

if(MSVC)
  target_compile_options(srf_tsf_tip PRIVATE /W3 /EHsc /utf-8)
  target_link_options(srf_tsf_tip PRIVATE "/DEF:${CMAKE_CURRENT_SOURCE_DIR}/srf_tsf_tip.def")
endif()

add_executable(srf_ime_overlay WIN32
  src/candidate_overlay_main.cpp
  src/candidate_overlay_placement.cpp
  src/candidate_window.cpp
  src/ime_config.cpp
)

target_include_directories(srf_ime_overlay PRIVATE "${CMAKE_CURRENT_SOURCE_DIR}/include")
target_compile_definitions(srf_ime_overlay PRIVATE UNICODE _UNICODE)
target_link_libraries(srf_ime_overlay PRIVATE
  ole32
  oleaut32
  uuid
  advapi32
  crypt32
  d2d1
  dwmapi
  dwrite
  user32
  shell32
  gdi32
  gdiplus
  msimg32
)

if(MSVC)
  set_property(TARGET srf_ime_overlay PROPERTY
    MSVC_RUNTIME_LIBRARY "MultiThreaded$<$<CONFIG:Debug>:Debug>")
  target_compile_options(srf_ime_overlay PRIVATE /W3 /EHsc /utf-8)
endif()
