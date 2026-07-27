use std::collections::BTreeMap;
use std::sync::Arc;

use lopdf::encryption::crypt_filters::{Aes128CryptFilter, CryptFilter};
use lopdf::{Document, EncryptionState, EncryptionVersion, Object, Permissions, StringFormat};
use rand::RngExt;

use crate::pdf::{file_label, validate_pdf_path};

/// The crypt filter name every reader expects for the standard security handler.
const FILTER_NAME: &[u8] = b"StdCF";

/// Password-protect `input` and write the result to `output`.
///
/// Uses AES-128 (encryption V4, revision 4). AES-256 (V5/R6) would be the
/// stronger choice on paper, but lopdf's V5 output does not interoperate: macOS
/// PDFKit accepts the password and then reads zero content out of the file,
/// which means the key it derives is not the one lopdf encrypted with. lopdf's
/// own V5 test only round-trips through lopdf, so it never caught this. V4
/// verifies clean against PDFKit, and a cipher no other reader can decrypt is
/// worse than a weaker one that works.
///
/// The same password is set as both the user and the owner password: this is a
/// "lock my file" tool, not a rights-management one, so anyone who can open the
/// document is also allowed to change it.
pub fn encrypt_pdf(input: &str, output: &str, password: &str) -> Result<(), String> {
    validate_pdf_path(input)?;
    if password.is_empty() {
        return Err("Enter a password.".into());
    }

    let mut doc = Document::load(input).map_err(|e| format!("Failed to read {}: {e}", file_label(input)))?;
    if doc.is_encrypted() {
        return Err(format!(
            "{} is already password-protected. Remove the existing password first.",
            file_label(input)
        ));
    }

    ensure_document_id(&mut doc);

    // V4 derives the file encryption key from the password and the document
    // itself, so the borrow of `doc` has to end before `encrypt` takes it
    // mutably — hence the block.
    let state = {
        let crypt_filter: Arc<dyn CryptFilter> = Arc::new(Aes128CryptFilter);
        let version = EncryptionVersion::V4 {
            document: &doc,
            encrypt_metadata: true,
            crypt_filters: BTreeMap::from([(FILTER_NAME.to_vec(), crypt_filter)]),
            stream_filter: FILTER_NAME.to_vec(),
            string_filter: FILTER_NAME.to_vec(),
            owner_password: password,
            user_password: password,
            permissions: Permissions::all(),
        };
        EncryptionState::try_from(version).map_err(|e| format!("Could not set up encryption: {e}"))?
    };

    doc.encrypt(&state)
        .map_err(|e| format!("Failed to encrypt {}: {e}", file_label(input)))?;

    doc.save(output)
        .map_err(|e| format!("Failed to write output: {e}"))?;
    Ok(())
}

/// Strip the password from `input`, writing a plain copy to `output`.
pub fn decrypt_pdf(input: &str, output: &str, password: &str) -> Result<(), String> {
    validate_pdf_path(input)?;

    if !is_encrypted(input)? {
        return Err(format!("{} is not password-protected.", file_label(input)));
    }

    // `Document::load` bails out of an encrypted file the moment the empty
    // password fails to authenticate, leaving the objects unread — the password
    // has to go in at load time, not after.
    let mut doc = Document::load_with_password(input, password).map_err(|_| "Wrong password.".to_string())?;

    // The objects are plaintext in memory now, but the trailer still advertises
    // the security handler. Left in place, the output would claim to be
    // encrypted while holding readable content, which no reader can open.
    if let Ok(encrypt_ref) = doc.trailer.get(b"Encrypt").and_then(Object::as_reference) {
        doc.objects.remove(&encrypt_ref);
    }
    doc.trailer.remove(b"Encrypt");
    doc.prune_objects();

    doc.save(output)
        .map_err(|e| format!("Failed to write output: {e}"))?;
    Ok(())
}

/// Give the document a file identifier if it has none.
///
/// The V4 key derivation hashes the first `/ID` element, and plenty of
/// generators — including lopdf itself — omit `/ID` entirely. Refusing to
/// encrypt those would be a confusing dead end, so mint one instead. Per spec
/// both elements are identical for a file that has never been updated.
fn ensure_document_id(doc: &mut Document) {
    if doc.trailer.get(b"ID").is_ok() {
        return;
    }

    let mut bytes = [0u8; 16];
    rand::rng().fill(&mut bytes);
    let id = Object::String(bytes.to_vec(), StringFormat::Hexadecimal);
    doc.trailer.set("ID", vec![id.clone(), id]);
}

/// Whether `path` is password-protected, so the UI can pick the right action.
pub fn is_encrypted(path: &str) -> Result<bool, String> {
    validate_pdf_path(path)?;
    let doc = Document::load(path).map_err(|e| format!("Failed to read {}: {e}", file_label(path)))?;
    Ok(doc.is_encrypted())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf::test_support::write_blank_pdf;

    /// A temp dir holding `plain.pdf` with `pages` blank pages, plus the paths
    /// the tests write their locked/unlocked copies to.
    struct Fixture {
        plain: String,
        locked: String,
        unlocked: String,
    }

    fn fixture(name: &str, pages: usize) -> Fixture {
        let dir = std::env::temp_dir().join(format!("paperblade_encrypt_{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");

        let path = |n: &str| dir.join(n).to_str().unwrap().to_string();
        let plain = path("plain.pdf");
        write_blank_pdf(&plain, pages);
        Fixture {
            plain,
            locked: path("locked.pdf"),
            unlocked: path("unlocked.pdf"),
        }
    }

    #[test]
    fn encrypting_then_decrypting_round_trips_the_pages() {
        let f = fixture("round_trip", 4);

        encrypt_pdf(&f.plain, &f.locked, "hunter2").expect("encrypt should succeed");
        assert!(is_encrypted(&f.locked).unwrap(), "output should be locked");

        decrypt_pdf(&f.locked, &f.unlocked, "hunter2").expect("decrypt should succeed");
        assert!(!is_encrypted(&f.unlocked).unwrap(), "output should be unlocked");

        let pages = Document::load(&f.unlocked).unwrap().get_pages().len();
        assert_eq!(pages, 4, "page count should survive the round trip");
    }

    #[test]
    fn encrypts_a_document_that_has_no_file_id() {
        let f = fixture("no_file_id", 1);

        // The blank fixture ships without a `/ID`, which is exactly the case
        // V4 key derivation cannot handle on its own.
        assert!(
            Document::load(&f.plain).unwrap().trailer.get(b"ID").is_err(),
            "fixture should have no /ID, otherwise this test proves nothing"
        );

        encrypt_pdf(&f.plain, &f.locked, "hunter2").expect("encrypt should mint an /ID");
        decrypt_pdf(&f.locked, &f.unlocked, "hunter2").expect("decrypt should succeed");
    }

    #[test]
    fn the_source_file_is_left_alone() {
        let f = fixture("source_intact", 2);

        encrypt_pdf(&f.plain, &f.locked, "hunter2").expect("encrypt should succeed");

        assert!(!is_encrypted(&f.plain).unwrap(), "source must stay plain");
    }

    #[test]
    fn rejects_the_wrong_password() {
        let f = fixture("wrong_password", 1);
        encrypt_pdf(&f.plain, &f.locked, "correct").expect("encrypt should succeed");

        let err = decrypt_pdf(&f.locked, &f.unlocked, "wrong").unwrap_err();
        assert!(err.contains("Wrong password"), "got {err}");
    }

    #[test]
    fn rejects_an_empty_password() {
        let f = fixture("empty_password", 1);
        let err = encrypt_pdf(&f.plain, &f.locked, "").unwrap_err();
        assert!(err.contains("Enter a password"), "got {err}");
    }

    #[test]
    fn refuses_to_double_encrypt() {
        let f = fixture("double_encrypt", 1);
        encrypt_pdf(&f.plain, &f.locked, "hunter2").expect("encrypt should succeed");

        let err = encrypt_pdf(&f.locked, &f.unlocked, "another").unwrap_err();
        assert!(err.contains("already password-protected"), "got {err}");
    }

    #[test]
    fn refuses_to_decrypt_a_plain_file() {
        let f = fixture("decrypt_plain", 1);
        let err = decrypt_pdf(&f.plain, &f.unlocked, "hunter2").unwrap_err();
        assert!(err.contains("not password-protected"), "got {err}");
    }
}
