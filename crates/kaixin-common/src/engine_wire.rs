pub const PIPE_NAME_ENV: &str = "SRF_ENGINE_PIPE_NAME";
pub const CAPABILITY_FILE_NAME: &str = "engine_capability.dat";
pub const CAPABILITY_MAGIC: &[u8] = b"KXIPC-DPAPI-1\n";
pub const PROTOCOL_MAGIC: u32 = 0x31504653;
pub const STORAGE_COMMAND: u16 = 16;
pub const MAX_STORAGE_REQUEST_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_STORAGE_RESPONSE_BYTES: usize = 128 * 1024 * 1024;

pub const PROTOCOL_VERSION: u16 = {
    let b = include_bytes!("../../../shared/engine_protocol_version.txt").trim_ascii();
    let mut i = 0;
    let mut version = 0_u16;
    while i < b.len() {
        assert!(b[i] >= b'0' && b[i] <= b'9');
        version = version * 10 + (b[i] - b'0') as u16;
        i += 1;
    }
    assert!(version != 0);
    version
};
