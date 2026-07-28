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
if(EXISTS "${CMAKE_CURRENT_SOURCE_DIR}/../VERSION")
  file(READ "${CMAKE_CURRENT_SOURCE_DIR}/../VERSION" SRF_APP_VERSION_FILE)
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

include(CTest)
if(BUILD_TESTING)
  add_executable(srf_candidate_result_stability_test
    tests/candidate_result_stability_test.cpp)
  target_include_directories(srf_candidate_result_stability_test PRIVATE
    "${CMAKE_CURRENT_SOURCE_DIR}/include")
  if(MSVC)
    target_compile_options(srf_candidate_result_stability_test PRIVATE /W3 /EHsc /utf-8)
  endif()
  add_test(NAME srf_candidate_result_stability
    COMMAND srf_candidate_result_stability_test)

  add_executable(srf_candidate_layout_policy_test
    tests/candidate_layout_policy_test.cpp)
  target_include_directories(srf_candidate_layout_policy_test PRIVATE
    "${CMAKE_CURRENT_SOURCE_DIR}/include")
  if(MSVC)
    target_compile_options(srf_candidate_layout_policy_test PRIVATE /W3 /EHsc /utf-8)
  endif()
  add_test(NAME srf_candidate_layout_policy
    COMMAND srf_candidate_layout_policy_test)

  add_executable(srf_app_options_match_test tests/app_options_match_test.cpp)
  target_include_directories(srf_app_options_match_test PRIVATE
    "${CMAKE_CURRENT_SOURCE_DIR}/include")
  if(MSVC)
    target_compile_options(srf_app_options_match_test PRIVATE /W3 /EHsc /utf-8)
  endif()
  add_test(NAME srf_app_options_match COMMAND srf_app_options_match_test)

  add_executable(srf_candidate_overlay_protocol_test
    tests/candidate_overlay_protocol_test.cpp)
  target_include_directories(srf_candidate_overlay_protocol_test PRIVATE
    "${CMAKE_CURRENT_SOURCE_DIR}/include")
  if(MSVC)
    target_compile_options(srf_candidate_overlay_protocol_test PRIVATE /W3 /EHsc /utf-8)
  endif()
  add_test(NAME srf_candidate_overlay_protocol
    COMMAND srf_candidate_overlay_protocol_test)

  add_executable(srf_candidate_overlay_policy_test
    tests/candidate_overlay_policy_test.cpp)
  target_include_directories(srf_candidate_overlay_policy_test PRIVATE
    "${CMAKE_CURRENT_SOURCE_DIR}/include")
  if(MSVC)
    target_compile_options(srf_candidate_overlay_policy_test PRIVATE /W3 /EHsc /utf-8)
  endif()
  add_test(NAME srf_candidate_overlay_policy
    COMMAND srf_candidate_overlay_policy_test)

  add_executable(srf_candidate_overlay_geometry_test
    tests/candidate_overlay_geometry_test.cpp)
  target_include_directories(srf_candidate_overlay_geometry_test PRIVATE
    "${CMAKE_CURRENT_SOURCE_DIR}/include")
  if(MSVC)
    target_compile_options(srf_candidate_overlay_geometry_test PRIVATE /W3 /EHsc /utf-8)
  endif()
  add_test(NAME srf_candidate_overlay_geometry
    COMMAND srf_candidate_overlay_geometry_test)

  add_executable(srf_game_overlay_config_test
    tests/game_overlay_config_test.cpp
    src/ime_config.cpp)
  target_include_directories(srf_game_overlay_config_test PRIVATE
    "${CMAKE_CURRENT_SOURCE_DIR}/include")
  target_compile_definitions(srf_game_overlay_config_test PRIVATE UNICODE _UNICODE)
  target_link_libraries(srf_game_overlay_config_test PRIVATE
    advapi32 crypt32 shell32 user32)
  if(MSVC)
    set_property(TARGET srf_game_overlay_config_test PROPERTY
      MSVC_RUNTIME_LIBRARY "MultiThreaded$<$<CONFIG:Debug>:Debug>")
    target_compile_options(srf_game_overlay_config_test PRIVATE /W3 /EHsc /utf-8)
  endif()
  add_test(NAME srf_game_overlay_config COMMAND srf_game_overlay_config_test)

  add_executable(srf_candidate_overlay_smoke_test WIN32
    tests/candidate_overlay_smoke_test.cpp
    src/candidate_overlay_client.cpp)
  target_include_directories(srf_candidate_overlay_smoke_test PRIVATE
    "${CMAKE_CURRENT_SOURCE_DIR}/include")
  target_compile_definitions(srf_candidate_overlay_smoke_test PRIVATE UNICODE _UNICODE)
  target_link_libraries(srf_candidate_overlay_smoke_test PRIVATE ole32 user32)
  add_dependencies(srf_candidate_overlay_smoke_test srf_ime_overlay)
  if(MSVC)
    set_property(TARGET srf_candidate_overlay_smoke_test PROPERTY
      MSVC_RUNTIME_LIBRARY "MultiThreaded$<$<CONFIG:Debug>:Debug>")
    target_compile_options(srf_candidate_overlay_smoke_test PRIVATE /W3 /EHsc /utf-8)
  endif()
  add_test(NAME srf_candidate_overlay_smoke COMMAND srf_candidate_overlay_smoke_test)
endif()
