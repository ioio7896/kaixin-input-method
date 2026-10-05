#include <windows.h>

#include <cstdlib>
#include <filesystem>
#include <iostream>
#include <string>

#include "diagnostic_privacy.h"
#include "overlapped_io.h"

void Require(bool condition, const char* message) {
  if (!condition) {
    std::cerr << message << " error=" << GetLastError() << '\n';
    std::exit(1);
  }
}

struct PipePair {
  HANDLE server = INVALID_HANDLE_VALUE;
  HANDLE client = INVALID_HANDLE_VALUE;
  explicit PipePair(unsigned index) {
    const std::wstring name = L"\\\\.\\pipe\\KaixinIoTest-" + std::to_wstring(GetCurrentProcessId()) +
                              L"-" + std::to_wstring(index);
    // This local test pipe carries only a fixed payload. An explicit DACL lets
    // both ordinary and restricted test runners connect to their own server.
    SECURITY_DESCRIPTOR descriptor = {};
    Require(InitializeSecurityDescriptor(&descriptor, SECURITY_DESCRIPTOR_REVISION) != FALSE,
            "initialize test descriptor");
    Require(SetSecurityDescriptorDacl(&descriptor, TRUE, nullptr, FALSE) != FALSE,
            "set test descriptor");
    SECURITY_ATTRIBUTES security = {sizeof(security), &descriptor, FALSE};
    server = CreateNamedPipeW(name.c_str(), PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED,
                              PIPE_TYPE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                              1, 4096, 4096, 0, &security);
    Require(server != INVALID_HANDLE_VALUE, "create pipe");
    auto connect = SrfOverlappedIo::Create(server, 1);
    Require(connect != nullptr, "create connect operation");
    const BOOL connected = ConnectNamedPipe(connect->handle, &connect->overlapped);
    const DWORD error = connected ? ERROR_SUCCESS : GetLastError();
    Require(connected || error == ERROR_IO_PENDING, "start connect");
    connect->pending = !connected;
    client = CreateFileW(name.c_str(), GENERIC_READ | GENERIC_WRITE, 0, nullptr, OPEN_EXISTING,
                         0, nullptr);
    Require(client != INVALID_HANDLE_VALUE, "open client");
    DWORD count = 0;
    Require(!connect->pending || connect->Await(1000, &count), "complete connect");
  }
  ~PipePair() {
    if (client != INVALID_HANDLE_VALUE) CloseHandle(client);
    if (server != INVALID_HANDLE_VALUE) CloseHandle(server);
  }
};

int main() {
  const std::wstring marker = L"KX_PRIVATE_MARKER_9a";
  const std::wstring redacted = RedactDiagnosticMessage(
      (L"reading=" + marker + L" current=" + marker + L" result=" + marker +
       L" reason=" + marker + L" status=" + marker + L" count=" + marker +
       L" process=" + marker + L" current_reading_units=12 request_id=42 status=ok hr=0x80004005").c_str());
  Require(redacted.find(marker) == std::wstring::npos, "private text must be absent");
  Require(redacted.find(L"current_reading_units=12") != std::wstring::npos, "keep length");
  Require(redacted.find(L"request_id=42") != std::wstring::npos, "keep request id");
  Require(redacted.find(L"status=ok") != std::wstring::npos, "keep enum");
  Require(redacted.find(L"hr=0x80004005") != std::wstring::npos, "keep HRESULT");
  Require(!IsNumericDiagnosticValue(L"12secret"), "reject numeric prefix");
  Require(!IsNumericDiagnosticValue(L"0x"), "reject empty hex");

  for (unsigned index = 0; index < 32; ++index) {
    PipePair pair(index);
    auto read = SrfOverlappedIo::Create(pair.server, 4);
    Require(read != nullptr, "create read operation");
    DWORD count = 0;
    Require(!ReadFile(read->handle, read->buffer.data(), 4, &count, &read->overlapped) &&
            GetLastError() == ERROR_IO_PENDING, "start pending read");
    read->pending = true;
    if (index % 2 == 0) {
      const char payload[] = "test";
      Require(WriteFile(pair.client, payload, 4, &count, nullptr) != FALSE, "write payload");
      Require(read->Await(1000, &count) && count == 4, "complete read");
      Require(std::string(read->buffer.begin(), read->buffer.end()) == "test", "owned buffer contents");
    } else {
      Require(!read->Await(1, &count) && GetLastError() == ERROR_TIMEOUT, "read timeout");
      HANDLE completion = nullptr;
      Require(DuplicateHandle(GetCurrentProcess(), read->overlapped.hEvent, GetCurrentProcess(),
                              &completion, 0, FALSE, DUPLICATE_SAME_ACCESS) != FALSE, "copy event");
      CloseHandle(pair.server);
      pair.server = INVALID_HANDLE_VALUE;
      read.reset();
      Require(WaitForSingleObject(completion, 1000) == WAIT_OBJECT_0, "cancel completes after caller leaves");
      CloseHandle(completion);
    }
  }
  const auto watchDir = std::filesystem::temp_directory_path() /
      (L"KaixinDirectoryIoTest-" + std::to_wstring(GetCurrentProcessId()));
  Require(CreateDirectoryW(watchDir.c_str(), nullptr) != FALSE, "create watch directory");
  for (unsigned index = 0; index < 8; ++index) {
    HANDLE directory = CreateFileW(watchDir.c_str(), FILE_LIST_DIRECTORY,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, nullptr, OPEN_EXISTING,
        FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED, nullptr);
    Require(directory != INVALID_HANDLE_VALUE, "open watch directory");
    auto watch = SrfOverlappedIo::Create(directory, 4096);
    CloseHandle(directory);
    Require(watch != nullptr, "own directory operation after original handle closes");
    Require(ReadDirectoryChangesW(watch->handle, watch->buffer.data(),
        static_cast<DWORD>(watch->buffer.size()), FALSE, FILE_NOTIFY_CHANGE_DIR_NAME,
        nullptr, &watch->overlapped, nullptr) != FALSE, "start directory watch");
    watch->pending = true;
    DWORD count = 0;
    if (index % 2 == 0) {
      const auto changedDir = watchDir / std::to_wstring(index);
      Require(CreateDirectoryW(changedDir.c_str(), nullptr) != FALSE, "change directory");
      Require(watch->Await(1000, &count) && count != 0, "complete directory watch");
      Require(RemoveDirectoryW(changedDir.c_str()) != FALSE, "remove changed directory");
    } else {
      Require(!watch->Await(1, &count) && GetLastError() == ERROR_TIMEOUT,
              "directory timeout");
      watch.reset();
    }
  }
  Require(RemoveDirectoryW(watchDir.c_str()) != FALSE, "remove watch directory");
  std::cout << "privacy and overlapped I/O tests passed\n";
  return 0;
}
