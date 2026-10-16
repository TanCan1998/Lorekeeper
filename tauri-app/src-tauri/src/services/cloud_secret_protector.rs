//! Windows 当前用户 DPAPI 保护器；只向上层返回通用错误码，不记录明文或原生错误详情。
//! 额外熵绑定 Lorekeeper、账户ID、规范端点与用户名，避免密文被换绑到另一端点。
//! 不使用 LOCAL_MACHINE，密文不随项目导出，其他平台明确不支持。
#[derive(Default)]
pub struct DpapiProtector;

pub trait SecretProtector {
    fn supported(&self) -> bool {
        true
    }
    fn encrypt(&self, plain: &[u8], context: &[u8]) -> Result<Vec<u8>, String>;
    fn decrypt(&self, cipher: &[u8], context: &[u8]) -> Result<Vec<u8>, String>;
}

impl SecretProtector for DpapiProtector {
    fn supported(&self) -> bool {
        cfg!(windows)
    }
    fn encrypt(&self, plain: &[u8], context: &[u8]) -> Result<Vec<u8>, String> {
        crypt(plain, context, true)
    }
    fn decrypt(&self, cipher: &[u8], context: &[u8]) -> Result<Vec<u8>, String> {
        crypt(cipher, context, false)
    }
}

#[cfg(not(windows))]
fn crypt(_: &[u8], _: &[u8], _: bool) -> Result<Vec<u8>, String> {
    Err("CLOUD_CREDENTIAL_PLATFORM_UNSUPPORTED".into())
}

#[cfg(windows)]
fn crypt(input: &[u8], context: &[u8], encrypt: bool) -> Result<Vec<u8>, String> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };
    struct Output(CRYPT_INTEGER_BLOB);
    impl Drop for Output {
        fn drop(&mut self) {
            if !self.0.pbData.is_null() {
                // SAFETY: 缓冲区由 DPAPI 通过 LocalAlloc 分配，长度由 API 返回。
                // 先逐字节易失擦除，再以匹配的 LocalFree 释放；不产生悬垂引用。
                unsafe {
                    for i in 0..self.0.cbData as usize {
                        std::ptr::write_volatile(self.0.pbData.add(i), 0);
                    }
                    LocalFree(self.0.pbData.cast());
                }
            }
        }
    }
    if input.is_empty() || input.len() > 128 * 1024 || context.len() > 8192 {
        return Err("CLOUD_CREDENTIAL_INPUT_INVALID".into());
    }
    let data = CRYPT_INTEGER_BLOB {
        cbData: input.len() as u32,
        pbData: input.as_ptr().cast_mut(),
    };
    let entropy = CRYPT_INTEGER_BLOB {
        cbData: context.len() as u32,
        pbData: context.as_ptr().cast_mut(),
    };
    let mut output = Output(CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    });
    // SAFETY: 输入切片在同步调用期间存活；输入与额外熵是只读 API 参数，
    // 输出指针有效；UI_FORBIDDEN 保证不会弹系统 UI 或在后台挂起。
    let ok = unsafe {
        if encrypt {
            CryptProtectData(
                &data,
                std::ptr::null(),
                &entropy,
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output.0,
            )
        } else {
            CryptUnprotectData(
                &data,
                std::ptr::null_mut(),
                &entropy,
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output.0,
            )
        }
    };
    if ok == 0 || output.0.pbData.is_null() || output.0.cbData == 0 || output.0.cbData > 128 * 1024
    {
        return Err("CLOUD_CREDENTIAL_UNAVAILABLE".into());
    }
    // SAFETY: 成功的 API 返回非空、有效 cbData 字节，复制后由 Output drop 擦除并释放。
    Ok(unsafe { std::slice::from_raw_parts(output.0.pbData, output.0.cbData as usize) }.to_vec())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[test]
    fn current_user_roundtrip_rejects_changed_context_and_ciphertext() {
        let protector = DpapiProtector;
        let plain = b"unit-test-only-not-a-real-credential";
        let cipher = protector.encrypt(plain, b"lorekeeper-account-1").unwrap();
        assert_ne!(&cipher, plain);
        assert_eq!(
            protector.decrypt(&cipher, b"lorekeeper-account-1").unwrap(),
            plain
        );
        assert!(protector.decrypt(&cipher, b"lorekeeper-account-2").is_err());
        let mut changed = cipher;
        let i = changed.len() / 2;
        changed[i] ^= 1;
        assert!(protector
            .decrypt(&changed, b"lorekeeper-account-1")
            .is_err());
    }
}
