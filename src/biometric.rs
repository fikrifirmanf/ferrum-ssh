//! Touch ID unlock for a master-password vault (macOS only).
//!
//! The master password is kept in the login keychain and only read back after the user
//! passes a Touch ID prompt. The prompt is a gate in front of the keychain item, not a
//! cryptographic key: it stops someone at an unlocked Mac from opening the vault, but
//! anything that can read your login keychain can read the password.

pub use imp::*;

#[cfg(target_os = "macos")]
mod imp {
    use std::sync::mpsc;

    use anyhow::{Result, anyhow, bail};
    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_foundation::{NSError, NSString};
    use objc2_local_authentication::{LAContext, LAError, LAPolicy};
    use security_framework::passwords::{
        delete_generic_password, get_generic_password, set_generic_password,
    };

    const SERVICE: &str = "dev.Ferrum.Ferrum";
    const ACCOUNT: &str = "vault-master-password";
    const POLICY: LAPolicy = LAPolicy::DeviceOwnerAuthenticationWithBiometrics;
    /// `errSecItemNotFound`
    const NOT_FOUND: i32 = -25300;

    pub const SUPPORTED: bool = true;
    pub const NAME: &str = "Touch ID";

    /// True when this Mac has Touch ID hardware with an enrolled fingerprint.
    pub fn available() -> bool {
        unsafe { LAContext::new().canEvaluatePolicy_error(POLICY).is_ok() }
    }

    /// True when a master password is stored for Touch ID unlock.
    pub fn enrolled() -> bool {
        use security_framework::item::{ItemClass, ItemSearchOptions};
        ItemSearchOptions::new()
            .class(ItemClass::generic_password())
            .service(SERVICE)
            .account(ACCOUNT)
            .load_data(false)
            .search()
            .is_ok()
    }

    /// Shows the Touch ID prompt and blocks until the user answers it.
    pub fn authenticate(reason: &str) -> Result<()> {
        let (tx, rx) = mpsc::channel::<std::result::Result<(), (isize, String)>>();
        let reply = RcBlock::new(move |ok: Bool, error: *mut NSError| {
            let outcome = if ok.as_bool() {
                Ok(())
            } else {
                let error = unsafe { error.as_ref() };
                Err(error
                    .map(|e| (e.code(), e.localizedDescription().to_string()))
                    .unwrap_or((0, "authentication failed".into())))
            };
            let _ = tx.send(outcome);
        });
        unsafe {
            LAContext::new().evaluatePolicy_localizedReason_reply(
                POLICY,
                &NSString::from_str(reason),
                &reply,
            );
        }
        match rx.recv() {
            Ok(Ok(())) => Ok(()),
            Ok(Err((code, _))) if code == LAError::UserCancel.0 || code == LAError::AppCancel.0 => {
                bail!("cancelled")
            }
            Ok(Err((_, message))) => Err(anyhow!(message)),
            Err(_) => bail!("Touch ID did not answer"),
        }
    }

    /// Stores `password` for later Touch ID unlock, replacing any earlier one.
    pub fn enroll(password: &str) -> Result<()> {
        set_generic_password(SERVICE, ACCOUNT, password.as_bytes())
            .map_err(|e| anyhow!("saving to the keychain failed: {e}"))
    }

    /// Asks for Touch ID, then returns the stored master password.
    pub fn unlock() -> Result<String> {
        authenticate("unlock your Ferrum vault")?;
        let bytes = get_generic_password(SERVICE, ACCOUNT)
            .map_err(|e| anyhow!("reading the keychain failed: {e}"))?;
        String::from_utf8(bytes).map_err(|_| anyhow!("stored password is not valid text"))
    }

    /// Forgets the stored password. Succeeds when there is none.
    pub fn forget() -> Result<()> {
        match delete_generic_password(SERVICE, ACCOUNT) {
            Err(e) if e.code() != NOT_FOUND => {
                Err(anyhow!("removing the keychain item failed: {e}"))
            }
            _ => Ok(()),
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use anyhow::{Result, bail};

    pub const SUPPORTED: bool = false;
    pub const NAME: &str = "biometrics";

    pub fn available() -> bool {
        false
    }
    pub fn enrolled() -> bool {
        false
    }
    pub fn authenticate(_reason: &str) -> Result<()> {
        bail!("not supported on this platform")
    }
    pub fn enroll(_password: &str) -> Result<()> {
        bail!("not supported on this platform")
    }
    pub fn unlock() -> Result<String> {
        bail!("not supported on this platform")
    }
    pub fn forget() -> Result<()> {
        Ok(())
    }
}
