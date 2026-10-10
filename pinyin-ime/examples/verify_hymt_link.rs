//! Exercise the actual IME translation client without changing the clipboard.
//! Run with HYMT_TRANSLATOR_EXE pointing to the HY-MT2 release executable.
use pinyin_ime::external_translation::{self, ExternalTranslationRequest};
use std::io::Read;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().nth(1).as_deref() == Some("send") {
        let mut payload = String::new();
        std::io::stdin()
            .take(1024 * 1024 + 1)
            .read_to_string(&mut payload)?;
        let request: ExternalTranslationRequest = serde_json::from_str(&payload)?;
        external_translation::send_request(&request)?;
        println!("accepted {}", request.request_id);
    } else {
        println!("{}", external_translation::test_connection()?);
    }
    Ok(())
}
