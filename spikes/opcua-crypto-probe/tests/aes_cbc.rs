//! AES-CBC with a caller-chosen IV and no padding (OPC UA derives key and IV from the channel
//! nonces and pads the message itself), against the NIST SP 800-38A vectors.

mod common;

use aws_lc_rs::cipher::{
    AES_128, AES_256, DecryptingKey, DecryptionContext, EncryptingKey, EncryptionContext,
    UnboundCipherKey,
};
use aws_lc_rs::iv::FixedLength;

const IV: &str = "000102030405060708090a0b0c0d0e0f";
const PLAINTEXT: &str = concat!(
    "6bc1bee22e409f96e93d7e117393172a",
    "ae2d8a571e03ac9c9eb76fac45af8e51",
    "30c81c46a35ce411e5fbc1191a0a52ef",
    "f69f2445df4f9b17ad2b417be66c3710",
);

fn iv() -> [u8; 16] {
    common::unhex(IV).try_into().expect("16 bytes")
}

fn encrypt(
    algorithm: &'static aws_lc_rs::cipher::Algorithm,
    key: &[u8],
    data: &[u8],
) -> Result<Vec<u8>, aws_lc_rs::error::Unspecified> {
    let key = EncryptingKey::cbc(UnboundCipherKey::new(algorithm, key)?)?;
    let mut buffer = data.to_vec();
    key.less_safe_encrypt(
        &mut buffer,
        EncryptionContext::Iv128(FixedLength::from(iv())),
    )?;
    Ok(buffer)
}

fn decrypt(
    algorithm: &'static aws_lc_rs::cipher::Algorithm,
    key: &[u8],
    data: &[u8],
) -> Result<Vec<u8>, aws_lc_rs::error::Unspecified> {
    let key = DecryptingKey::cbc(UnboundCipherKey::new(algorithm, key)?)?;
    let mut buffer = data.to_vec();
    let plain_len = key
        .decrypt(
            &mut buffer,
            DecryptionContext::Iv128(FixedLength::from(iv())),
        )?
        .len();
    buffer.truncate(plain_len);
    Ok(buffer)
}

#[test]
fn aes128_cbc_matches_sp800_38a_f_2_1() {
    let key = common::unhex("2b7e151628aed2a6abf7158809cf4f3c");
    let expected = concat!(
        "7649abac8119b246cee98e9b12e9197d",
        "5086cb9b507219ee95db113a917678b2",
        "73bed6b8e3c1743b7116e69e22229516",
        "3ff1caa1681fac09120eca307586e1a7",
    );
    let plaintext = common::unhex(PLAINTEXT);
    let ciphertext = encrypt(&AES_128, &key, &plaintext).expect("encrypt");
    assert_eq!(common::hex(&ciphertext), expected);
    // No padding: the output is exactly as long as the input.
    assert_eq!(ciphertext.len(), plaintext.len());
    assert_eq!(
        decrypt(&AES_128, &key, &ciphertext).expect("decrypt"),
        plaintext
    );
}

#[test]
fn aes256_cbc_matches_sp800_38a_f_2_5() {
    let key = common::unhex("603deb1015ca71be2b73aef0857d77811f352c073b6108d72d9810a30914dff4");
    let expected = concat!(
        "f58c4c04d6e5f1ba779eabfb5f7bfbd6",
        "9cfc4e967edb808d679f777bc6702c7d",
        "39f23369a9d9bacfa530e26304231461",
        "b2eb05e2c39be9fcda6c19078c6a9d1b",
    );
    let plaintext = common::unhex(PLAINTEXT);
    let ciphertext = encrypt(&AES_256, &key, &plaintext).expect("encrypt");
    assert_eq!(common::hex(&ciphertext), expected);
    assert_eq!(
        decrypt(&AES_256, &key, &ciphertext).expect("decrypt"),
        plaintext
    );
}

#[test]
fn input_that_is_not_whole_blocks_is_refused() {
    let key = common::unhex("2b7e151628aed2a6abf7158809cf4f3c");
    assert!(encrypt(&AES_128, &key, &[0u8; 17]).is_err());
    assert!(encrypt(&AES_128, &key, &[0u8; 15]).is_err());
    assert!(decrypt(&AES_128, &key, &[0u8; 17]).is_err());
}

#[test]
fn a_key_of_the_wrong_length_is_refused() {
    assert!(UnboundCipherKey::new(&AES_128, &[0u8; 32]).is_err());
    assert!(UnboundCipherKey::new(&AES_256, &[0u8; 16]).is_err());
}

#[test]
fn the_iv_matters() {
    let key = common::unhex("2b7e151628aed2a6abf7158809cf4f3c");
    let plaintext = common::unhex(PLAINTEXT);
    let one = encrypt(&AES_128, &key, &plaintext).expect("encrypt");
    let other_key =
        EncryptingKey::cbc(UnboundCipherKey::new(&AES_128, &key).expect("key")).expect("cbc");
    let mut buffer = plaintext.clone();
    other_key
        .less_safe_encrypt(
            &mut buffer,
            EncryptionContext::Iv128(FixedLength::from([0xaa; 16])),
        )
        .expect("encrypt");
    assert_ne!(one, buffer);
}
