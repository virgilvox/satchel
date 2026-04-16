use anyhow::Result;
use std::path::{Path, PathBuf};

/// Validate that a vault name contains only safe filesystem characters.
fn validate_vault_name(name: &str) -> Result<()> {
    if name.is_empty() {
        anyhow::bail!("Vault name cannot be empty");
    }
    if name.len() > 64 {
        anyhow::bail!("Vault name cannot exceed 64 characters");
    }
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        anyhow::bail!(
            "Vault name can only contain letters, numbers, hyphens, and underscores"
        );
    }
    if name.starts_with('-') || name.starts_with('_') {
        anyhow::bail!("Vault name must start with a letter or number");
    }
    Ok(())
}

pub fn list_vaults(base_path: &Path) -> Result<()> {
    let vaults_dir = base_path.join("vaults");

    if !vaults_dir.exists() {
        println!("No vaults found. Create one with: satchel init <name>");
        return Ok(());
    }

    let active = get_active_vault(base_path).unwrap_or_default();

    println!("Vaults:");
    for entry in std::fs::read_dir(&vaults_dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            let name = entry.file_name().to_string_lossy().to_string();
            let marker = if name == active { " (active)" } else { "" };
            let db_path = entry.path().join("satchel.db");
            let size = if db_path.exists() {
                humanize_bytes(std::fs::metadata(&db_path)?.len())
            } else {
                "empty".to_string()
            };
            println!("  {name}{marker} [{size}]");
        }
    }

    Ok(())
}

pub fn create_vault(base_path: &Path, name: &str) -> Result<()> {
    validate_vault_name(name)?;

    let vault_dir = base_path.join("vaults").join(name);

    if vault_dir.exists() {
        anyhow::bail!("Vault '{name}' already exists");
    }

    std::fs::create_dir_all(&vault_dir)?;
    std::fs::create_dir_all(vault_dir.join("inbox"))?;

    let _db = crate::rag::Database::open(&vault_dir)?;

    println!("[satchel] Created vault: {name}");
    println!("  Path:  {}", vault_dir.display());
    println!("  Inbox: {}/inbox/", vault_dir.display());

    let vaults_dir = base_path.join("vaults");
    let vault_count = std::fs::read_dir(&vaults_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .count();

    if vault_count == 1 {
        set_active(base_path, name)?;
    }

    Ok(())
}

pub fn set_active(base_path: &Path, name: &str) -> Result<()> {
    let vault_dir = base_path.join("vaults").join(name);
    if !vault_dir.exists() {
        anyhow::bail!("Vault '{name}' does not exist");
    }

    let config_path = base_path.join("satchel.toml");
    std::fs::write(&config_path, format!("active_vault = \"{name}\"\n"))?;

    println!("[satchel] Active vault: {name}");
    Ok(())
}

fn get_active_vault(base_path: &Path) -> Option<String> {
    let config_path = base_path.join("satchel.toml");
    let content = std::fs::read_to_string(&config_path).ok()?;

    for line in content.lines() {
        if let Some(val) = line.strip_prefix("active_vault") {
            let val = val.trim().trim_start_matches('=').trim().trim_matches('"');
            return Some(val.to_string());
        }
    }
    None
}

pub fn active_vault_path(base_path: &Path) -> Result<PathBuf> {
    let name = get_active_vault(base_path)
        .ok_or_else(|| anyhow::anyhow!("No active vault. Run: satchel init <name>"))?;
    let path = base_path.join("vaults").join(&name);
    if !path.exists() {
        anyhow::bail!("Active vault '{name}' directory missing. Run: satchel init {name}");
    }
    Ok(path)
}

fn humanize_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB"];
    let mut size = bytes as f64;
    for unit in UNITS {
        if size < 1024.0 {
            return format!("{:.1} {unit}", size);
        }
        size /= 1024.0;
    }
    format!("{:.1} TB", size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_vault_name_valid() {
        assert!(validate_vault_name("personal").is_ok());
        assert!(validate_vault_name("work-notes").is_ok());
        assert!(validate_vault_name("project_alpha").is_ok());
        assert!(validate_vault_name("v2").is_ok());
    }

    #[test]
    fn test_validate_vault_name_empty() {
        assert!(validate_vault_name("").is_err());
    }

    #[test]
    fn test_validate_vault_name_special_chars() {
        assert!(validate_vault_name("my vault").is_err());
        assert!(validate_vault_name("../escape").is_err());
        assert!(validate_vault_name("foo/bar").is_err());
        assert!(validate_vault_name("test\"quote").is_err());
    }

    #[test]
    fn test_validate_vault_name_leading_special() {
        assert!(validate_vault_name("-leading").is_err());
        assert!(validate_vault_name("_leading").is_err());
    }

    #[test]
    fn test_validate_vault_name_too_long() {
        let name = "a".repeat(65);
        assert!(validate_vault_name(&name).is_err());
        let name = "a".repeat(64);
        assert!(validate_vault_name(&name).is_ok());
    }
}
