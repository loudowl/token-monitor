use anyhow::{Context, Result};

const SERVICE: &str = "token-monitor";

pub fn set_secret(provider: &str, value: &str) -> Result<()> {
    let entry = keyring::Entry::new(SERVICE, provider).context("keyring entry")?;
    entry.set_password(value).context("set keyring password")?;
    Ok(())
}

pub fn get_secret(provider: &str) -> Result<Option<String>> {
    let entry = keyring::Entry::new(SERVICE, provider).context("keyring entry")?;
    match entry.get_password() {
        Ok(v) => Ok(Some(v)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn delete_secret(provider: &str) -> Result<()> {
    let entry = keyring::Entry::new(SERVICE, provider).context("keyring entry")?;
    let _ = entry.delete_credential();
    Ok(())
}
