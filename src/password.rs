// Lobby passwords (Extension/Multiplayer/Session/password.cpp): PBKDF2-HMAC-SHA256 key per
// session, and an HMAC proof per join. The password itself never crosses the wire.
use crate::protocol::PROTOCOL_VERSION;
use hmac::{Hmac, Mac};
use sha2::Sha256;

pub type PasswordKey = [u8; 32];

pub fn password_key(password: &str, session: u64) -> Option<PasswordKey> {
    if password.is_empty() {
        return None;
    }
    if password.len() > 64 || session == 0 {
        panic!("Lobby passwords must be 1-64 UTF-8 bytes.");
    }
    let mut salt = [0u8; 16];
    salt[..8].copy_from_slice(&[b'R', b'e', b'S', b'k', b'a', b't', b'e', PROTOCOL_VERSION as u8]);
    salt[8..].copy_from_slice(&session.to_le_bytes());
    let mut key = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<Sha256>(password.as_bytes(), &salt, 100_000, &mut key);
    Some(key)
}

#[allow(clippy::too_many_arguments)]
pub fn password_proof(
    key: &PasswordKey,
    session: u64,
    map: u64,
    host: u64,
    guest: u64,
    host_epoch: u64,
    guest_epoch: u64,
    challenge: u64,
) -> PasswordKey {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key).expect("HMAC takes any key length");
    let mut message = b"ReSkateProof".to_vec();
    message.push(PROTOCOL_VERSION as u8);
    for v in [session, map, host, guest, host_epoch, guest_epoch, challenge] {
        message.extend_from_slice(&v.to_le_bytes());
    }
    mac.update(&message);
    mac.finalize().into_bytes().into()
}

pub fn proof_matches(a: &PasswordKey, b: &PasswordKey) -> bool {
    let mut difference = 0u8;
    for i in 0..a.len() {
        difference |= a[i] ^ b[i];
    }
    std::hint::black_box(difference) == 0
}
