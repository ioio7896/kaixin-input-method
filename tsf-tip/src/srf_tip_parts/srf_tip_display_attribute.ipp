namespace {

STDMETHODIMP CSrfDisplayAttributeInfo::GetGUID(GUID* guid) {
  if (!guid) return E_POINTER;
  *guid = GUID_DISPLAY_ATTRIBUTE_SRF_INPUT;
  return S_OK;
}

STDMETHODIMP CSrfDisplayAttributeInfo::GetDescription(BSTR* description) {
  if (!description) return E_POINTER;
  *description = SysAllocString(L"\u5f00\u5fc3\u8f93\u5165\u6cd5 Display Attribute");
  return *description ? S_OK : E_OUTOFMEMORY;
}

STDMETHODIMP CSrfDisplayAttributeInfo::GetAttributeInfo(TF_DISPLAYATTRIBUTE* attr) {
  if (!attr) return E_POINTER;
  *attr = m_attr;
  return S_OK;
}

STDMETHODIMP CSrfDisplayAttributeInfo::SetAttributeInfo(const TF_DISPLAYATTRIBUTE* attr) {
  if (!attr) return E_POINTER;
  m_attr = *attr;
  return S_OK;
}

STDMETHODIMP CSrfDisplayAttributeInfo::Reset() {
  m_attr = DefaultDisplayAttribute();
  return S_OK;
}

STDMETHODIMP CSrfEnumDisplayAttributeInfo::QueryInterface(REFIID riid, void** ppv) {
  if (!ppv) return E_POINTER;
  *ppv = nullptr;
  if (riid == IID_IUnknown || riid == IID_IEnumTfDisplayAttributeInfo) {
    *ppv = static_cast<IEnumTfDisplayAttributeInfo*>(this);
    AddRef();
    return S_OK;
  }
  return E_NOINTERFACE;
}

STDMETHODIMP_(ULONG) CSrfEnumDisplayAttributeInfo::Release() {
  const ULONG count = InterlockedDecrement(&m_cRef);
  if (count == 0) delete this;
  return count;
}

STDMETHODIMP CSrfEnumDisplayAttributeInfo::Clone(IEnumTfDisplayAttributeInfo** ppEnum) {
  if (!ppEnum) return E_POINTER;
  *ppEnum = new (std::nothrow) CSrfEnumDisplayAttributeInfo(m_index);
  return *ppEnum ? S_OK : E_OUTOFMEMORY;
}

STDMETHODIMP CSrfEnumDisplayAttributeInfo::Next(ULONG count, ITfDisplayAttributeInfo** info,
                                                ULONG* fetched) {
  if (fetched) *fetched = 0;
  if (!info) return E_POINTER;

  ULONG produced = 0;
  while (produced < count && m_index == 0) {
    ITfDisplayAttributeInfo* current = new (std::nothrow) CSrfDisplayAttributeInfo();
    if (!current) return E_OUTOFMEMORY;
    info[produced++] = current;
    ++m_index;
  }

  if (fetched) *fetched = produced;
  return produced == count ? S_OK : S_FALSE;
}

STDMETHODIMP CSrfEnumDisplayAttributeInfo::Reset() {
  m_index = 0;
  return S_OK;
}

STDMETHODIMP CSrfEnumDisplayAttributeInfo::Skip(ULONG count) {
  m_index = std::min<ULONG>(1, m_index + count);
  return count == 0 || m_index < 1 ? S_OK : S_FALSE;
}

}  // namespace
