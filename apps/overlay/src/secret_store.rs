//! Windows DPAPI-backed storage for the cloud API key.
//!
//! The encrypted blob can only be decrypted by the same Windows user account.
//! No plaintext credential is written to disk.

use std::path::PathBuf;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{LocalFree, HLOCAL};
use windows::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
};

fn secret_path() -> Result<PathBuf, String> {
    dirs::config_dir()
        .map(|dir| dir.join("lingxi").join("cloud-key.dpapi"))
        .ok_or_else(|| "无法解析灵犀配置目录".to_string())
}

fn copy_and_free(blob: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    let bytes = if blob.pbData.is_null() || blob.cbData == 0 {
        Vec::new()
    } else {
        // SAFETY: DPAPI returned a valid buffer of cbData bytes.
        unsafe { std::slice::from_raw_parts(blob.pbData, blob.cbData as usize).to_vec() }
    };
    // SAFETY: DPAPI allocates output buffers with LocalAlloc.
    if !blob.pbData.is_null() {
        unsafe {
            let _ = LocalFree(HLOCAL(blob.pbData.cast()));
        }
    }
    bytes
}

fn protect_bytes(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut input_bytes = data.to_vec();
    let input = CRYPT_INTEGER_BLOB {
        cbData: input_bytes
            .len()
            .try_into()
            .map_err(|_| "API Key 长度超出限制".to_string())?,
        pbData: input_bytes.as_mut_ptr(),
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    // SAFETY: input points to `input_bytes` for the duration of the call;
    // output is copied and released with LocalFree below.
    unsafe {
        CryptProtectData(
            &input,
            PCWSTR::null(),
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
        .map_err(|error| format!("Windows DPAPI 加密失败: {error}"))?;
    }
    let encrypted = copy_and_free(output);
    if encrypted.is_empty() {
        Err("Windows DPAPI 返回了空密文".into())
    } else {
        Ok(encrypted)
    }
}

fn unprotect_bytes(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut input_bytes = data.to_vec();
    let input = CRYPT_INTEGER_BLOB {
        cbData: input_bytes
            .len()
            .try_into()
            .map_err(|_| "加密凭据长度超出限制".to_string())?,
        pbData: input_bytes.as_mut_ptr(),
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    // SAFETY: input points to `input_bytes`; output is copied and released.
    unsafe {
        CryptUnprotectData(
            &input,
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
        .map_err(|error| format!("Windows DPAPI 解密失败: {error}"))?;
    }
    Ok(copy_and_free(output))
}

pub fn save_api_key(secret: &str) -> Result<(), String> {
    if secret.is_empty() {
        return delete_api_key();
    }
    let encrypted = protect_bytes(secret.as_bytes())?;
    let path = secret_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    std::fs::write(path, encrypted).map_err(|error| error.to_string())
}

pub fn load_api_key() -> Result<Option<String>, String> {
    let path = secret_path()?;
    let encrypted = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    if encrypted.is_empty() {
        return Ok(None);
    }
    let plain = unprotect_bytes(&encrypted)?;
    let secret = String::from_utf8(plain).map_err(|_| "解密后的 API Key 不是 UTF-8".to_string())?;
    Ok((!secret.is_empty()).then_some(secret))
}

pub fn delete_api_key() -> Result<(), String> {
    let path = secret_path()?;
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dpapi_round_trip_does_not_touch_persistent_credentials() {
        let sample = format!("test-key-{}", std::process::id());
        let encrypted = protect_bytes(sample.as_bytes()).unwrap();
        assert_ne!(encrypted, sample.as_bytes());
        let plain = unprotect_bytes(&encrypted).unwrap();
        assert_eq!(plain, sample.as_bytes());
    }
}
