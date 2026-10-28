use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};

use parking_lot::RwLock;
use thiserror::Error;
use uuid::Uuid;

#[path = "atomic_file.rs"]
pub(crate) mod atomic_file;

use crate::local_secrets::LocalEncryptedStore;
use crate::models::{
    ActionDefinition, ActionKind, AppSettings, CreateProviderInput, ProviderConfig,
    PublicSettings, SettingsUpdate, UpdateProviderInput, WindowSize, SETTINGS_VERSION,
};

const SECRET_ACCOUNT_PREFIX: &str = "provider-api-key:";
const MAX_API_KEY_LENGTH: usize = 16_384;

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error("无法读取或保存设置：{0}")]
    Io(#[from] std::io::Error),
    #[error("设置文件格式无效：{0}")]
    Json(#[from] serde_json::Error),
    #[error("设置内容无效：{0}")]
    Validation(String),
    #[error("找不到服务商：{0}")]
    ProviderNotFound(String),
    #[error("无法访问本地加密密钥存储")]
    SecretStorage,
}

/// A deliberately small abstraction so repository tests can use an isolated
/// in-memory backend while production stores encrypted values outside settings.json.
pub trait SecretStore: Send + Sync {
    fn get(&self, account: &str) -> Result<Option<String>, SettingsError>;
    fn set(&self, account: &str, secret: &str) -> Result<(), SettingsError>;
    fn delete(&self, account: &str) -> Result<(), SettingsError>;
}

pub struct SettingsRepository {
    path: PathBuf,
    state: RwLock<AppSettings>,
    secrets: Arc<dyn SecretStore>,
}

impl std::fmt::Debug for SettingsRepository {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SettingsRepository")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl SettingsRepository {
    pub fn new(path: impl Into<PathBuf>) -> Result<Self, SettingsError> {
        let path = path.into();
        let secrets = Arc::new(LocalEncryptedStore::for_settings_file(&path)?);
        Self::with_secret_store(path, secrets)
    }

    pub fn with_secret_store(
        path: impl Into<PathBuf>,
        secrets: Arc<dyn SecretStore>,
    ) -> Result<Self, SettingsError> {
        let path = path.into();
        let loaded = load_settings(&path)?;
        let repository = Self {
            path,
            state: RwLock::new(loaded.settings),
            secrets,
        };

        if loaded.must_persist {
            repository.persist(&repository.state.read())?;
        }
        Ok(repository)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn get_settings(&self) -> AppSettings {
        self.state.read().clone()
    }

    pub fn get_public_settings(&self) -> Result<PublicSettings, SettingsError> {
        let settings = self.state.read().clone();
        let mut status = Vec::with_capacity(settings.providers.len());
        for provider in &settings.providers {
            status.push((
                provider.id.clone(),
                self.secrets
                    .get(&secret_account(&provider.id))?
                    .is_some_and(|value| !value.trim().is_empty()),
            ));
        }
        Ok(PublicSettings::from_settings(&settings, |id| {
            status
                .iter()
                .find(|(provider_id, _)| provider_id == id)
                .is_some_and(|(_, configured)| *configured)
        }))
    }

    pub fn update_result_last_size(
        &self,
        size: WindowSize,
    ) -> Result<PublicSettings, SettingsError> {
        self.transact(|settings| {
            if settings.result.remember_size {
                settings.result.last_size = Some(size);
            }
            Ok(())
        })?;
        self.get_public_settings()
    }

    pub fn update_capture_preferences(&self, color: String, line_width: u8) -> Result<(), SettingsError> {
        self.transact(|settings| {
            settings.capture.color = color;
            settings.capture.line_width = line_width;
            Ok(())
        })
    }

    pub fn clear_result_last_size(&self) -> Result<PublicSettings, SettingsError> {
        self.transact(|settings| {
            settings.result.last_size = None;
            Ok(())
        })?;
        self.get_public_settings()
    }

    pub fn update(&self, update: SettingsUpdate) -> Result<PublicSettings, SettingsError> {
        self.transact_with_provider_cleanup(|settings| {
            if let Some(value) = update.tray_enabled {
                settings.tray_enabled = value;
            }
            if let Some(value) = update.quick_ask_shortcut {
                settings.quick_ask_shortcut = value;
            }
            if let Some(value) = update.capture { settings.capture = value; }
            if let Some(value) = update.default_ai {
                settings.default_ai = value;
            }
            if let Some(value) = update.locale {
                settings.locale = value;
            }
            if let Some(value) = update.translate {
                settings.translate = value;
            }
            if let Some(mut value) = update.result {
                // Window dimensions are runtime-owned state. A settings page
                // can stay open with a stale public snapshot while a result
                // window is resized, so an ordinary form save must not write
                // that stale value back over the latest persisted size.
                value.last_size = settings.result.last_size;
                settings.result = value;
            }
            if let Some(value) = update.filter {
                settings.filter = value;
            }
            if let Some(value) = update.providers {
                settings.providers = value;
            }
            if let Some(mut value) = update.actions {
                let mut fixed_ask = settings
                    .actions
                    .iter()
                    .find(|action| action.kind == ActionKind::Ask)
                    .cloned();
                value.retain(|action| action.kind != ActionKind::Ask);
                reconcile_fixed_ask_route(&mut fixed_ask);
                if let Some(fixed_ask) = fixed_ask {
                    value.push(fixed_ask);
                }
                settings.actions = value;
            }
            Ok(())
        })?;
        self.get_public_settings()
    }

    /// Replaces all non-secret settings with a renderer-provided public copy.
    /// `keyConfigured` values are intentionally ignored.
    pub fn replace_public(&self, public: PublicSettings) -> Result<PublicSettings, SettingsError> {
        self.transact_with_provider_cleanup(|settings| {
            let fixed_ask = settings
                .actions
                .iter()
                .find(|action| action.kind == ActionKind::Ask)
                .cloned();
            let mut actions = public.actions;
            actions.retain(|action| action.kind != ActionKind::Ask);
            let providers = public
                .providers
                .into_iter()
                .map(|provider| ProviderConfig {
                    id: provider.id,
                    name: provider.name,
                    kind: provider.kind,
                    dsh_workspace: provider.dsh_workspace,
                    enabled: provider.enabled,
                    base_url: provider.base_url,
                    models: provider.models,
                })
                .collect::<Vec<_>>();
            let mut fixed_ask = fixed_ask;
            reconcile_fixed_ask_route(&mut fixed_ask);
            if let Some(fixed_ask) = fixed_ask {
                actions.push(fixed_ask);
            }
            *settings = AppSettings {
                version: SETTINGS_VERSION,
                tray_enabled: public.tray_enabled,
                quick_ask_shortcut: public.quick_ask_shortcut,
                capture: public.capture,
                default_ai: public.default_ai,
                locale: public.locale,
                translate: public.translate,
                result: public.result,
                filter: public.filter,
                providers,
                actions,
            };
            Ok(())
        })?;
        self.get_public_settings()
    }

    pub fn create_provider(
        &self,
        input: CreateProviderInput,
    ) -> Result<PublicSettings, SettingsError> {
        let provider_id = format!("provider-{}", Uuid::new_v4().simple());
        self.transact(|settings| {
            settings.providers.push(ProviderConfig {
                id: provider_id,
                name: input.name,
                kind: input.kind,
                dsh_workspace: input.dsh_workspace,
                enabled: true,
                base_url: input.base_url,
                models: Vec::new(),
            });
            Ok(())
        })?;
        self.get_public_settings()
    }

    pub fn update_provider(
        &self,
        provider_id: &str,
        update: UpdateProviderInput,
    ) -> Result<PublicSettings, SettingsError> {
        let provider_id = provider_id.to_owned();
        self.transact(|settings| {
            let valid_models = {
                let provider = settings
                    .providers
                    .iter_mut()
                    .find(|provider| provider.id == provider_id)
                    .ok_or_else(|| SettingsError::ProviderNotFound(provider_id.clone()))?;
                if let Some(name) = update.name {
                    provider.name = name;
                }
                if let Some(base_url) = update.base_url {
                    provider.base_url = base_url;
                }
                if let Some(kind) = update.kind {
                    provider.kind = kind;
                }
                if let Some(workspace) = update.dsh_workspace {
                    provider.dsh_workspace = Some(workspace);
                }
                update.models.map(|models| {
                    provider.models = models;
                    provider
                        .models
                        .iter()
                        .map(|model| model.id.clone())
                        .collect::<std::collections::HashSet<_>>()
                })
            };
            if let Some(valid_models) = valid_models {
                if settings.default_ai.provider_id == provider_id
                    && !valid_models.contains(&settings.default_ai.model_id)
                {
                    settings.default_ai = Default::default();
                }
                for action in &mut settings.actions {
                    if action.provider_id.as_deref() == Some(provider_id.as_str())
                        && action
                            .model_id
                            .as_deref()
                            .is_some_and(|model| !model.is_empty() && !valid_models.contains(model))
                    {
                        action.model_id = Some(String::new());
                    }
                }
            }
            Ok(())
        })?;
        self.get_public_settings()
    }

    pub fn delete_provider(&self, provider_id: &str) -> Result<PublicSettings, SettingsError> {
        let provider_id = provider_id.to_owned();
        self.transact_with_provider_cleanup(|settings| {
            if !settings
                .providers
                .iter()
                .any(|provider| provider.id == provider_id)
            {
                return Err(SettingsError::ProviderNotFound(provider_id.clone()));
            }
            settings
                .providers
                .retain(|provider| provider.id != provider_id);
            if settings.default_ai.provider_id == provider_id {
                settings.default_ai = Default::default();
            }
            for action in &mut settings.actions {
                if action.provider_id.as_deref() == Some(provider_id.as_str()) {
                    action.provider_id = Some(String::new());
                    action.model_id = Some(String::new());
                }
            }
            Ok(())
        })?;
        self.get_public_settings()
    }

    pub fn set_provider_api_key(
        &self,
        provider_id: &str,
        api_key: &str,
    ) -> Result<PublicSettings, SettingsError> {
        // Hold the writer lock across ensure + secret write so concurrent
        // delete_provider cannot remove the provider between check and set,
        // leaving an orphan secret entry.
        {
            let state = self.state.write();
            if !state
                .providers
                .iter()
                .any(|provider| provider.id == provider_id)
            {
                return Err(SettingsError::ProviderNotFound(provider_id.to_owned()));
            }
            let api_key = api_key.trim();
            if api_key.is_empty() || api_key.len() > MAX_API_KEY_LENGTH {
                return Err(SettingsError::Validation(
                    "API Key 应为 1–16384 个字符".to_owned(),
                ));
            }
            self.secrets.set(&secret_account(provider_id), api_key)?;
        }
        self.get_public_settings()
    }

    pub fn clear_provider_api_key(
        &self,
        provider_id: &str,
    ) -> Result<PublicSettings, SettingsError> {
        // Same write-lock scope as set_provider_api_key / delete_provider:
        // ensure and secret delete must not race with provider lifecycle.
        {
            let state = self.state.write();
            if !state
                .providers
                .iter()
                .any(|provider| provider.id == provider_id)
            {
                return Err(SettingsError::ProviderNotFound(provider_id.to_owned()));
            }
            self.secrets.delete(&secret_account(provider_id))?;
        }
        self.get_public_settings()
    }

    pub fn get_provider(&self, provider_id: &str) -> Result<ProviderConfig, SettingsError> {
        self.state
            .read()
            .providers
            .iter()
            .find(|provider| provider.id == provider_id)
            .cloned()
            .ok_or_else(|| SettingsError::ProviderNotFound(provider_id.to_owned()))
    }

    pub(crate) fn eudic_authorization(&self) -> Result<Option<String>, SettingsError> {
        self.secrets.get("integration:eudic")
    }

    pub(crate) fn set_eudic_authorization(&self, value: &str) -> Result<(), String> {
        let value = value.trim();
        if value.len() > 4096 || value.contains(['\r', '\n']) { return Err("授权信息格式无效".to_owned()); }
        if value.is_empty() { return self.secrets.delete("integration:eudic").map_err(|e| e.to_string()); }
        self.secrets.set("integration:eudic", value).map_err(|e| e.to_string())
    }

    pub(crate) fn get_api_key(&self, provider_id: &str) -> Result<Option<String>, SettingsError> {
        self.ensure_provider(provider_id)?;
        self.secrets.get(&secret_account(provider_id))
    }

    fn ensure_provider(&self, provider_id: &str) -> Result<(), SettingsError> {
        if self
            .state
            .read()
            .providers
            .iter()
            .any(|provider| provider.id == provider_id)
        {
            Ok(())
        } else {
            Err(SettingsError::ProviderNotFound(provider_id.to_owned()))
        }
    }

    fn transact<F>(&self, operation: F) -> Result<(), SettingsError>
    where
        F: FnOnce(&mut AppSettings) -> Result<(), SettingsError>,
    {
        // Hold the writer guard through persistence so concurrent Tauri
        // commands cannot both derive from the same stale snapshot and lose
        // one another's changes.
        let mut state = self.state.write();
        let mut candidate = state.clone();
        operation(&mut candidate)?;
        candidate = candidate
            .normalize_and_validate()
            .map_err(SettingsError::Validation)?;
        if candidate == *state {
            return Ok(());
        }
        self.persist(&candidate)?;
        *state = candidate;
        Ok(())
    }

    fn transact_with_provider_cleanup<F>(&self, operation: F) -> Result<(), SettingsError>
    where
        F: FnOnce(&mut AppSettings) -> Result<(), SettingsError>,
    {
        // Hold the settings writer lock across both stores. This prevents a
        // provider key from being recreated concurrently between validation,
        // encrypted-secret cleanup and the JSON commit.
        let mut state = self.state.write();
        let previous = state.clone();
        let mut candidate = previous.clone();
        operation(&mut candidate)?;
        candidate = candidate
            .normalize_and_validate()
            .map_err(SettingsError::Validation)?;

        let retained = candidate
            .providers
            .iter()
            .map(|provider| provider.id.as_str())
            .collect::<std::collections::HashSet<_>>();
        let removed_accounts = previous
            .providers
            .iter()
            .filter(|provider| !retained.contains(provider.id.as_str()))
            .map(|provider| secret_account(&provider.id))
            .collect::<Vec<_>>();
        let secret_snapshots = removed_accounts
            .iter()
            .map(|account| {
                self.secrets
                    .get(account)
                    .map(|secret| (account.clone(), secret))
            })
            .collect::<Result<Vec<_>, _>>()?;

        self.persist(&candidate)?;
        for account in &removed_accounts {
            if let Err(delete_error) = self.secrets.delete(account) {
                let restore_result = self.restore_secret_snapshots(&secret_snapshots);
                if let Err(rollback_error) = self.persist(&previous) {
                    // The candidate is still the durable JSON state. Keep the
                    // in-memory copy aligned with it even though cleanup failed.
                    *state = candidate;
                    return Err(rollback_error);
                }
                return match restore_result {
                    Ok(()) => Err(delete_error),
                    Err(restore_error) => Err(restore_error),
                };
            }
        }
        *state = candidate;
        Ok(())
    }

    fn restore_secret_snapshots(
        &self,
        snapshots: &[(String, Option<String>)],
    ) -> Result<(), SettingsError> {
        let mut first_error = None;
        for (account, secret) in snapshots {
            let result = match secret {
                Some(secret) => self.secrets.set(account, secret),
                None => self.secrets.delete(account),
            };
            if first_error.is_none() {
                first_error = result.err();
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    fn persist(&self, settings: &AppSettings) -> Result<(), SettingsError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(settings)?;
        let temporary = self
            .path
            .with_extension(format!("{}.tmp", Uuid::new_v4().simple()));
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let write_result = (|| -> Result<(), std::io::Error> {
            let mut file = options.open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            // Windows refuses ReplaceFileW/MoveFileExW while this process
            // still owns an open handle to the replacement file.
            drop(file);
            atomic_file::replace_file(&temporary, &self.path)?;
            Ok(())
        })();
        if write_result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        write_result.map_err(SettingsError::Io)
    }
}

/// Ask resolves the shared default at request time, with no hidden per-action override.
fn reconcile_fixed_ask_route(fixed_ask: &mut Option<ActionDefinition>) {
    if let Some(ask) = fixed_ask {
        ask.provider_id = Some(String::new());
        ask.model_id = Some(String::new());
        ask.thinking_mode = crate::models::ThinkingMode::Off;
        ask.enabled = true;
    }
}

fn secret_account(provider_id: &str) -> String {
    format!("{SECRET_ACCOUNT_PREFIX}{provider_id}")
}

struct LoadedSettings {
    settings: AppSettings,
    must_persist: bool,
}

fn load_settings(path: &Path) -> Result<LoadedSettings, SettingsError> {
    if !path.exists() {
        return Ok(LoadedSettings {
            settings: AppSettings::default()
                .normalize_and_validate()
                .map_err(SettingsError::Validation)?,
            must_persist: true,
        });
    }
    let bytes = fs::read(path)?;
    if bytes.len() > 5 * 1024 * 1024 {
        return Err(SettingsError::Validation("设置文件体积异常".to_owned()));
    }
    let mut settings: AppSettings = serde_json::from_slice(&bytes)?;
    if settings.version != SETTINGS_VERSION {
        return Err(SettingsError::Validation(
            "设置文件版本不受当前应用支持".to_owned(),
        ));
    }
    let persisted = settings.clone();
    // Repair dangling model references without migrating historical formats.
    heal_orphan_provider_references(&mut settings);
    let settings = settings
        .normalize_and_validate()
        .map_err(SettingsError::Validation)?;
    Ok(LoadedSettings {
        must_persist: settings != persisted,
        settings,
    })
}

fn heal_orphan_provider_references(settings: &mut AppSettings) {
    let providers_map: std::collections::HashMap<&str, &crate::models::ProviderConfig> = settings
        .providers
        .iter()
        .map(|provider| (provider.id.as_str(), provider))
        .collect();

    for action in &mut settings.actions {
        if action.kind == ActionKind::Ask {
            action.provider_id = Some(String::new());
            action.model_id = Some(String::new());
            action.thinking_mode = crate::models::ThinkingMode::Off;
        } else {
            let provider_id = action.provider_id.as_deref().unwrap_or("").trim();
            let model_id = action.model_id.as_deref().unwrap_or("").trim();

            if !provider_id.is_empty() {
                match providers_map.get(provider_id) {
                    Some(provider) => {
                        if !model_id.is_empty()
                            && !provider.models.iter().any(|model| model.id == model_id)
                        {
                            action.model_id = Some(String::new());
                        }
                    }
                    None => {
                        action.provider_id = Some(String::new());
                        action.model_id = Some(String::new());
                    }
                }
            } else if !model_id.is_empty() {
                action.model_id = Some(String::new());
            }
        }
    }

    let default_provider_id = settings.default_ai.provider_id.trim();
    let default_model_id = settings.default_ai.model_id.trim();
    if !default_provider_id.is_empty() || !default_model_id.is_empty() {
        let valid = providers_map.get(default_provider_id).is_some_and(|provider| {
            provider.models.iter().any(|model| model.id == default_model_id)
        });
        if !valid {
            settings.default_ai = Default::default();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Mutex};

    use tempfile::tempdir;

    use super::*;
    use crate::models::{
        ProviderModel, ThinkingCapability, ThinkingCapabilitySource, ThinkingDialect, ThinkingLevel,
        DEFAULT_PROVIDER_ID,
    };

    #[derive(Default)]
    struct MemorySecrets(Mutex<HashMap<String, String>>);

    impl SecretStore for MemorySecrets {
        fn get(&self, account: &str) -> Result<Option<String>, SettingsError> {
            Ok(self.0.lock().unwrap().get(account).cloned())
        }

        fn set(&self, account: &str, secret: &str) -> Result<(), SettingsError> {
            self.0
                .lock()
                .unwrap()
                .insert(account.to_owned(), secret.to_owned());
            Ok(())
        }

        fn delete(&self, account: &str) -> Result<(), SettingsError> {
            self.0.lock().unwrap().remove(account);
            Ok(())
        }
    }

    #[derive(Default)]
    struct DeleteFailingSecrets(Mutex<HashMap<String, String>>);

    impl SecretStore for DeleteFailingSecrets {
        fn get(&self, account: &str) -> Result<Option<String>, SettingsError> {
            Ok(self.0.lock().unwrap().get(account).cloned())
        }

        fn set(&self, account: &str, secret: &str) -> Result<(), SettingsError> {
            self.0
                .lock()
                .unwrap()
                .insert(account.to_owned(), secret.to_owned());
            Ok(())
        }

        fn delete(&self, _account: &str) -> Result<(), SettingsError> {
            Err(SettingsError::SecretStorage)
        }
    }

    fn configure_ai_repository(repository: &SettingsRepository, model_id: &str, api_key: &str) {
        let mut settings = repository.get_settings();
        settings.providers[0].name = "Configured Provider".to_owned();
        settings.providers[0].base_url = "https://example.com/v1".to_owned();
        settings.providers[0].models = vec![ProviderModel {
            id: model_id.to_owned(),
            name: model_id.to_owned(),
            thinking_levels: Vec::new(),
            thinking_capability: None,
        }];
        for action in settings
            .actions
            .iter_mut()
            .filter(|action| action.kind.is_ai())
        {
            action.provider_id = Some(DEFAULT_PROVIDER_ID.to_owned());
            action.model_id = Some(model_id.to_owned());
        }
        repository
            .update(SettingsUpdate {
                default_ai: Some(crate::models::DefaultAiModel {
                    provider_id: DEFAULT_PROVIDER_ID.into(),
                    model_id: model_id.into(),
                }),
                providers: Some(settings.providers),
                actions: Some(settings.actions),
                ..Default::default()
            })
            .unwrap();
        repository
            .set_provider_api_key(DEFAULT_PROVIDER_ID, api_key)
            .unwrap();
    }

    #[test]
    fn current_settings_and_secrets_reopen_without_rewriting_or_reseeding() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let repository = SettingsRepository::new(&path).unwrap();
        configure_ai_repository(&repository, "current-model", "sk-current");
        repository
            .set_eudic_authorization("current-authorization")
            .unwrap();
        let mut actions = repository.get_settings().actions;
        actions[0].prompt = Some("自定义翻译：{{text}}".to_owned());
        repository
            .update(SettingsUpdate {
                actions: Some(actions),
                default_ai: Some(Default::default()),
                tray_enabled: Some(false),
                ..Default::default()
            })
            .unwrap();
        let expected = repository.get_settings();
        let original = fs::read(&path).unwrap();
        let encrypted_path = path.with_file_name(crate::local_secrets::ENCRYPTED_SECRETS_FILE_NAME);
        let encrypted = fs::read(&encrypted_path).unwrap();
        drop(repository);

        let reopened = SettingsRepository::new(&path).unwrap();
        assert_eq!(reopened.get_settings(), expected);
        assert_eq!(
            reopened
                .get_api_key(DEFAULT_PROVIDER_ID)
                .unwrap()
                .as_deref(),
            Some("sk-current")
        );
        assert_eq!(
            reopened.eudic_authorization().unwrap().as_deref(),
            Some("current-authorization")
        );
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(fs::read(&encrypted_path).unwrap(), encrypted);
    }

    #[test]
    fn unsupported_settings_are_rejected_without_modifying_the_file() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let current = serde_json::to_value(AppSettings::default()).unwrap();
        let mut invalid = Vec::new();
        for version in (1..SETTINGS_VERSION).chain([SETTINGS_VERSION + 1]) {
            let mut value = current.clone();
            value["version"] = version.into();
            invalid.push(value);
        }
        for field in ["version", "trayEnabled", "quickAskShortcut", "defaultAi", "capture"] {
            let mut value = current.clone();
            value.as_object_mut().unwrap().remove(field);
            invalid.push(value);
        }
        let mut value = current.clone();
        value["selectionCapture"] = serde_json::json!({});
        invalid.push(value);
        let mut value = current.clone();
        let action = value["actions"][0].as_object_mut().unwrap();
        let kind = action.remove("kind").unwrap();
        action.insert("type".to_owned(), kind);
        invalid.push(value);
        let mut value = current.clone();
        value["filter"] = serde_json::json!({"mode": "default", "list": []});
        invalid.push(value);
        let mut value = current.clone();
        value["capture"]["unexpected"] = true.into();
        invalid.push(value);
        let mut value = current;
        value["providers"][0]
            .as_object_mut()
            .unwrap()
            .remove("kind");
        invalid.push(value);
        for value in invalid {
            let original = serde_json::to_vec_pretty(&value).unwrap();
            fs::write(&path, &original).unwrap();
            assert!(SettingsRepository::new(&path).is_err());
            assert_eq!(fs::read(&path).unwrap(), original);
        }
    }

    #[test]
    fn default_ai_persists_without_action_overrides_and_clears_on_model_removal() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let secrets = Arc::new(MemorySecrets::default());
        let repository = SettingsRepository::with_secret_store(&path, secrets.clone()).unwrap();
        configure_ai_repository(&repository, "shared-model", "key");
        let route = crate::models::DefaultAiModel {
            provider_id: DEFAULT_PROVIDER_ID.into(),
            model_id: "shared-model".into(),
        };
        repository
            .update(SettingsUpdate {
                default_ai: Some(route.clone()),
                ..Default::default()
            })
            .unwrap();
        let reopened = SettingsRepository::with_secret_store(&path, secrets.clone()).unwrap();
        assert_eq!(reopened.get_public_settings().unwrap().default_ai, route);
        reopened
            .update_provider(
                DEFAULT_PROVIDER_ID,
                UpdateProviderInput {
                    models: Some(vec![]),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(reopened.get_settings().default_ai, Default::default());
        assert_eq!(
            SettingsRepository::with_secret_store(&path, secrets)
                .unwrap()
                .get_settings()
                .default_ai,
            Default::default()
        );
    }

    #[test]
    fn quick_ask_shortcut_survives_restart() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let original = AppSettings::default();
        fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();
        let secrets = Arc::new(MemorySecrets::default());
        let repository = SettingsRepository::with_secret_store(&path, secrets.clone()).unwrap();
        assert_eq!(
            repository.get_settings().quick_ask_shortcut,
            "Control+Shift+Space"
        );
        assert_eq!(repository.get_settings().providers, original.providers);
        assert!(repository
            .update(SettingsUpdate {
                quick_ask_shortcut: Some("Space".into()),
                ..Default::default()
            })
            .is_err());
        repository
            .update(SettingsUpdate {
                quick_ask_shortcut: Some("Alt+KeyK".into()),
                ..Default::default()
            })
            .unwrap();
        drop(repository);
        let reopened = SettingsRepository::with_secret_store(&path, secrets).unwrap();
        assert_eq!(
            reopened.get_public_settings().unwrap().quick_ask_shortcut,
            "Alt+KeyK"
        );
        assert_eq!(reopened.get_settings().version, SETTINGS_VERSION);
    }

    #[test]
    fn tray_preference_survives_restart() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("settings.json");
        fs::write(&path, serde_json::to_vec(&AppSettings::default()).unwrap()).unwrap();
        let secrets = Arc::new(MemorySecrets::default());
        let repository = SettingsRepository::with_secret_store(&path, secrets.clone()).unwrap();
        assert!(repository.get_public_settings().unwrap().tray_enabled);
        let persisted: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(persisted["trayEnabled"], true);
        let public = repository
            .update(SettingsUpdate {
                tray_enabled: Some(false),
                ..Default::default()
            })
            .unwrap();
        assert!(!public.tray_enabled);
        drop(repository);
        let reopened = SettingsRepository::with_secret_store(&path, secrets).unwrap();
        assert!(!reopened.get_settings().tray_enabled);
        let mut public = reopened.get_public_settings().unwrap();
        public.tray_enabled = true;
        assert!(reopened.replace_public(public).unwrap().tray_enabled);
    }

    #[test]
    fn api_keys_are_never_written_to_json_or_public_settings() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let repository =
            SettingsRepository::with_secret_store(&path, Arc::new(MemorySecrets::default()))
                .unwrap();
        let public = repository
            .set_provider_api_key(DEFAULT_PROVIDER_ID, "sk-super-secret")
            .unwrap();
        assert!(public.providers[0].key_configured);
        let json = fs::read_to_string(path).unwrap();
        assert!(!json.contains("sk-super-secret"));
        assert!(!json.contains("apiKey"));
    }

    #[test]
    fn eudic_authorization_is_encrypted_and_never_public() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let repository = SettingsRepository::new(&path).unwrap();
        repository
            .set_eudic_authorization("NIS test-eudic-secret")
            .unwrap();
        assert!(!fs::read_to_string(&path)
            .unwrap()
            .contains("test-eudic-secret"));
        assert!(
            !serde_json::to_string(&repository.get_public_settings().unwrap())
                .unwrap()
                .contains("test-eudic-secret")
        );
        drop(repository);
        let reopened = SettingsRepository::new(&path).unwrap();
        assert_eq!(
            reopened.eudic_authorization().unwrap().as_deref(),
            Some("NIS test-eudic-secret")
        );
        assert!(reopened
            .set_eudic_authorization("NIS test\r\ninjected: header")
            .is_err());
        reopened.set_eudic_authorization("").unwrap();
        assert_eq!(reopened.eudic_authorization().unwrap(), None);
    }

    #[test]
    fn fresh_repository_has_only_the_default_openai_address_and_no_api_key() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let repository = SettingsRepository::new(&path).unwrap();

        let settings = repository.get_settings();
        assert_eq!(settings.providers.len(), 1);
        assert_eq!(settings.providers[0].id, DEFAULT_PROVIDER_ID);
        assert_eq!(settings.providers[0].base_url, "https://api.openai.com/v1");
        assert!(settings.providers[0].models.is_empty());
        assert_eq!(repository.get_api_key(DEFAULT_PROVIDER_ID).unwrap(), None);
        assert!(!repository.get_public_settings().unwrap().providers[0].key_configured);

        let persisted = fs::read_to_string(path).unwrap();
        assert!(!persisted.contains("apiKey"));
        assert!(!directory
            .path()
            .join(crate::local_secrets::ENCRYPTED_SECRETS_FILE_NAME)
            .exists());
    }

    #[test]
    fn default_repository_encrypts_api_keys_locally_and_reopens_them() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let repository = SettingsRepository::new(&path).unwrap();
        repository
            .set_provider_api_key(DEFAULT_PROVIDER_ID, "sk-encrypted-locally")
            .unwrap();
        drop(repository);

        let encrypted_path = directory
            .path()
            .join(crate::local_secrets::ENCRYPTED_SECRETS_FILE_NAME);
        let key_path = directory
            .path()
            .join(crate::local_secrets::LOCAL_SECRET_KEY_FILE_NAME);
        assert!(!fs::read_to_string(&path)
            .unwrap()
            .contains("sk-encrypted-locally"));
        assert!(!fs::read_to_string(&encrypted_path)
            .unwrap()
            .contains("sk-encrypted-locally"));
        assert_eq!(fs::read(&key_path).unwrap().len(), 32);

        let reopened = SettingsRepository::new(path).unwrap();
        assert_eq!(
            reopened
                .get_api_key(DEFAULT_PROVIDER_ID)
                .unwrap()
                .as_deref(),
            Some("sk-encrypted-locally")
        );
        assert!(reopened.get_public_settings().unwrap().providers[0].key_configured);
    }

    #[test]
    fn set_api_key_fails_if_provider_missing() {
        let directory = tempdir().unwrap();
        let secrets = Arc::new(MemorySecrets::default());
        let repository = SettingsRepository::with_secret_store(
            directory.path().join("settings.json"),
            secrets.clone(),
        )
        .unwrap();

        let err = repository.set_provider_api_key("gone", "sk-x").unwrap_err();
        assert!(matches!(
            err,
            SettingsError::ProviderNotFound(ref id) if id == "gone"
        ));
        assert!(secrets.get(&secret_account("gone")).unwrap().is_none());
    }

    #[test]
    fn set_api_key_after_delete_fails_and_does_not_leave_orphan_secret() {
        let directory = tempdir().unwrap();
        let secrets = Arc::new(MemorySecrets::default());
        let repository = SettingsRepository::with_secret_store(
            directory.path().join("settings.json"),
            secrets.clone(),
        )
        .unwrap();
        repository
            .set_provider_api_key(DEFAULT_PROVIDER_ID, "sk-before-delete")
            .unwrap();
        repository.delete_provider(DEFAULT_PROVIDER_ID).unwrap();

        let err = repository
            .set_provider_api_key(DEFAULT_PROVIDER_ID, "sk-orphan")
            .unwrap_err();
        assert!(matches!(
            err,
            SettingsError::ProviderNotFound(ref id) if id == DEFAULT_PROVIDER_ID
        ));
        assert!(secrets
            .get(&secret_account(DEFAULT_PROVIDER_ID))
            .unwrap()
            .is_none());
    }

    #[test]
    fn clear_api_key_fails_if_provider_missing() {
        let directory = tempdir().unwrap();
        let repository = SettingsRepository::with_secret_store(
            directory.path().join("settings.json"),
            Arc::new(MemorySecrets::default()),
        )
        .unwrap();

        let err = repository.clear_provider_api_key("gone").unwrap_err();
        assert!(matches!(
            err,
            SettingsError::ProviderNotFound(ref id) if id == "gone"
        ));
    }

    #[test]
    fn provider_deletion_unbinds_actions() {
        let directory = tempdir().unwrap();
        let secrets = Arc::new(MemorySecrets::default());
        let repository = SettingsRepository::with_secret_store(
            directory.path().join("settings.json"),
            secrets.clone(),
        )
        .unwrap();
        repository
            .set_provider_api_key(DEFAULT_PROVIDER_ID, "sk-to-delete")
            .unwrap();
        let public = repository.delete_provider(DEFAULT_PROVIDER_ID).unwrap();
        assert!(public.providers.is_empty());
        assert!(public
            .actions
            .iter()
            .filter(|action| action.kind.is_ai())
            .all(|action| action.provider_id.as_deref() == Some("")));
        assert!(public
            .actions
            .iter()
            .filter(|action| action.kind.is_ai())
            .all(|action| action.model_id.as_deref() == Some("")));
        assert!(secrets
            .get(&secret_account(DEFAULT_PROVIDER_ID))
            .unwrap()
            .is_none());
    }

    #[test]
    fn provider_deletion_rolls_back_when_secret_cleanup_fails() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let repository =
            SettingsRepository::with_secret_store(&path, Arc::new(DeleteFailingSecrets::default()))
                .unwrap();
        repository
            .set_provider_api_key(DEFAULT_PROVIDER_ID, "sk-must-survive")
            .unwrap();
        let before = fs::read_to_string(&path).unwrap();

        assert!(matches!(
            repository.delete_provider(DEFAULT_PROVIDER_ID),
            Err(SettingsError::SecretStorage)
        ));
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
        let public = repository.get_public_settings().unwrap();
        assert_eq!(public.providers.len(), 1);
        assert!(public.providers[0].key_configured);
        assert!(public
            .actions
            .iter()
            .filter(|action| action.kind.is_ai())
            .all(|action| action.provider_id.as_deref() == Some(DEFAULT_PROVIDER_ID)));
    }

    #[test]
    fn full_settings_update_rolls_back_provider_removal_when_secret_cleanup_fails() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let repository =
            SettingsRepository::with_secret_store(&path, Arc::new(DeleteFailingSecrets::default()))
                .unwrap();
        repository
            .set_provider_api_key(DEFAULT_PROVIDER_ID, "sk-must-survive")
            .unwrap();
        let before = fs::read_to_string(&path).unwrap();
        let actions = repository
            .get_settings()
            .actions
            .into_iter()
            .map(|mut action| {
                if action.kind.is_ai() {
                    action.provider_id = Some(String::new());
                    action.model_id = Some(String::new());
                }
                action
            })
            .collect();

        assert!(matches!(
            repository.update(SettingsUpdate {
                providers: Some(Vec::new()),
                actions: Some(actions),
                ..Default::default()
            }),
            Err(SettingsError::SecretStorage)
        ));
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
        let public = repository.get_public_settings().unwrap();
        assert_eq!(public.providers.len(), 1);
        assert!(public.providers[0].key_configured);
    }

    #[test]
    fn replacing_provider_list_removes_orphaned_secret_entries() {
        let directory = tempdir().unwrap();
        let secrets = Arc::new(MemorySecrets::default());
        let repository = SettingsRepository::with_secret_store(
            directory.path().join("settings.json"),
            secrets.clone(),
        )
        .unwrap();
        repository
            .set_provider_api_key(DEFAULT_PROVIDER_ID, "sk-to-remove")
            .unwrap();

        repository
            .update(SettingsUpdate {
                providers: Some(Vec::new()),
                actions: Some(
                    repository
                        .get_settings()
                        .actions
                        .into_iter()
                        .map(|mut action| {
                            if action.kind.is_ai() {
                                action.provider_id = Some(String::new());
                                action.model_id = Some(String::new());
                            }
                            action
                        })
                        .collect(),
                ),
                ..Default::default()
            })
            .unwrap();

        assert!(secrets
            .get(&secret_account(DEFAULT_PROVIDER_ID))
            .unwrap()
            .is_none());
    }

    #[test]
    fn unchanged_capture_preferences_do_not_access_the_destination() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let repository =
            SettingsRepository::with_secret_store(&path, Arc::new(MemorySecrets::default()))
                .unwrap();
        let before = repository.get_settings();
        // A directory cannot be replaced by the settings file. An unchanged
        // update must succeed without attempting that replacement.
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        for _ in 0..10 {
            repository
                .update_capture_preferences(before.capture.color.clone(), before.capture.line_width)
                .unwrap();
        }
        assert_eq!(repository.get_settings(), before);
        assert!(path.is_dir());
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn unchanged_normalized_transactions_preserve_file_bytes() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let repository =
            SettingsRepository::with_secret_store(&path, Arc::new(MemorySecrets::default()))
                .unwrap();
        let original = format!("\n{}\n", fs::read_to_string(&path).unwrap());
        fs::write(&path, &original).unwrap();
        repository
            .transact(|settings| {
                settings.providers[0].name = format!(" {} ", settings.providers[0].name);
                Ok(())
            })
            .unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
    }

    #[test]
    fn capture_preference_changes_validate_and_commit_only_after_successful_persistence() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let repository =
            SettingsRepository::with_secret_store(&path, Arc::new(MemorySecrets::default()))
                .unwrap();
        let before = repository.get_settings();
        let original = fs::read(&path).unwrap();
        assert!(matches!(
            repository.update_capture_preferences("invalid".into(), 3),
            Err(SettingsError::Validation(_))
        ));
        assert_eq!(fs::read(&path).unwrap(), original);
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(matches!(
            repository.update_capture_preferences("#123456".into(), 8),
            Err(SettingsError::Io(_))
        ));
        assert_eq!(repository.get_settings(), before);
        fs::remove_dir(&path).unwrap();
        fs::write(&path, original).unwrap();
        repository
            .update_capture_preferences("#123456".into(), 8)
            .unwrap();
        let saved: AppSettings = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved, repository.get_settings());
        assert_eq!(saved.capture.color, "#123456");
        assert_eq!(saved.capture.line_width, 8);
        assert_eq!(saved.providers, before.providers);
    }

    #[test]
    fn concurrent_capture_preferences_and_result_sizes_preserve_both_updates() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let repository = Arc::new(
            SettingsRepository::with_secret_store(&path, Arc::new(MemorySecrets::default()))
                .unwrap(),
        );
        let barrier = Arc::new(std::sync::Barrier::new(2));
        std::thread::scope(|scope| {
            let capture_repository = repository.clone();
            let capture_barrier = barrier.clone();
            scope.spawn(move || {
                capture_barrier.wait();
                for _ in 0..16 {
                    capture_repository
                        .update_capture_preferences("#123456".into(), 8)
                        .unwrap();
                }
            });
            barrier.wait();
            repository
                .update_result_last_size(WindowSize {
                    width: 812.0,
                    height: 536.0,
                })
                .unwrap();
        });
        let saved: AppSettings = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved, repository.get_settings());
        assert_eq!(saved.capture.color, "#123456");
        assert_eq!(saved.capture.line_width, 8);
        assert_eq!(
            saved.result.last_size,
            Some(WindowSize {
                width: 812.0,
                height: 536.0
            })
        );
    }

    #[test]
    fn failed_validation_does_not_change_file_or_memory() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let repository =
            SettingsRepository::with_secret_store(&path, Arc::new(MemorySecrets::default()))
                .unwrap();
        let before = fs::read_to_string(&path).unwrap();
        let result = repository.update(SettingsUpdate {
            translate: Some(crate::models::TranslationSettings {
                primary_language: crate::models::TranslationLanguage::ZhCn,
                alternate_language: crate::models::TranslationLanguage::ZhCn,
                ..Default::default()
            }),
            ..Default::default()
        });
        assert!(result.is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), before);
    }

    #[test]
    fn ordinary_settings_updates_preserve_runtime_owned_result_size() {
        let directory = tempdir().unwrap();
        let repository = SettingsRepository::with_secret_store(
            directory.path().join("settings.json"),
            Arc::new(MemorySecrets::default()),
        )
        .unwrap();
        let remembered = WindowSize {
            width: 812.0,
            height: 536.0,
        };
        repository.update_result_last_size(remembered).unwrap();

        let mut stale_result = repository.get_settings().result;
        stale_result.opacity = 0.8;
        stale_result.last_size = None;
        let saved = repository
            .update(SettingsUpdate {
                result: Some(stale_result),
                ..Default::default()
            })
            .unwrap();

        assert_eq!(saved.result.opacity, 0.8);
        assert_eq!(saved.result.last_size, Some(remembered));
        let reset = repository.clear_result_last_size().unwrap();
        assert_eq!(reset.result.last_size, None);
    }

    #[test]
    fn settings_round_trip_preserves_optional_thinking_capability() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let repository = SettingsRepository::new(&path).unwrap();
        let mut settings = repository.get_settings();
        settings.providers[0].models = vec![ProviderModel {
            id: "o3-mini".to_owned(),
            name: "o3-mini".to_owned(),
            thinking_levels: vec![ThinkingLevel::Low, ThinkingLevel::Medium],
            thinking_capability: Some(ThinkingCapability {
                source: ThinkingCapabilitySource::Explicit,
                dialect: Some(ThinkingDialect::ReasoningEffort),
                supports_off: false,
            }),
        }];
        repository
            .update(SettingsUpdate {
                providers: Some(settings.providers),
                ..Default::default()
            })
            .unwrap();
        drop(repository);
        let reopened = SettingsRepository::new(&path).unwrap();
        assert_eq!(reopened.get_settings().version, SETTINGS_VERSION);
        assert_eq!(
            reopened.get_settings().providers[0].models[0].thinking_capability,
            Some(ThinkingCapability {
                source: ThinkingCapabilitySource::Explicit,
                dialect: Some(ThinkingDialect::ReasoningEffort),
                supports_off: false,
            })
        );
    }

    #[test]
    fn custom_provider_only_startup_does_not_panic_or_fail_on_ask_action() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let custom_provider = ProviderConfig {
            id: "deepseek".to_owned(),
            name: "DeepSeek".to_owned(),
            kind: crate::models::ProviderKind::Openai,
            dsh_workspace: None,
            enabled: true,
            base_url: "https://api.deepseek.com/v1".to_owned(),
            models: vec![ProviderModel {
                id: "deepseek-chat".to_owned(),
                name: "deepseek-chat".to_owned(),
                thinking_levels: Vec::new(),
                thinking_capability: None,
            }],
        };
        let mut settings = AppSettings::default();
        settings.providers = vec![custom_provider];
        settings.default_ai = crate::models::DefaultAiModel {
            provider_id: "deepseek".to_owned(),
            model_id: "deepseek-chat".to_owned(),
        };
        for action in &mut settings.actions {
            if action.kind.is_ai() && action.kind != ActionKind::Ask {
                action.provider_id = Some("deepseek".to_owned());
                action.model_id = Some("deepseek-chat".to_owned());
            }
        }
        let normalized = settings.normalize_and_validate().unwrap();
        fs::write(&path, serde_json::to_string_pretty(&normalized).unwrap()).unwrap();

        let loaded = load_settings(&path).expect("Settings should load without error");
        let ask = loaded
            .settings
            .actions
            .iter()
            .find(|a| a.kind == ActionKind::Ask)
            .expect("Ask action must be present");
        assert_eq!(ask.provider_id.as_deref(), Some(""));
        assert_eq!(ask.model_id.as_deref(), Some(""));
    }

    #[test]
    fn orphan_provider_and_model_references_are_healed_on_load() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let custom_provider = ProviderConfig {
            id: "custom".to_owned(),
            name: "Custom".to_owned(),
            kind: crate::models::ProviderKind::Openai,
            dsh_workspace: None,
            enabled: true,
            base_url: "https://api.custom.com/v1".to_owned(),
            models: vec![ProviderModel {
                id: "model-1".to_owned(),
                name: "model-1".to_owned(),
                thinking_levels: Vec::new(),
                thinking_capability: None,
            }],
        };
        let mut settings = AppSettings::default();
        settings.providers = vec![custom_provider];
        settings.default_ai = crate::models::DefaultAiModel {
            provider_id: "non-existent-provider".to_owned(),
            model_id: "non-existent-model".to_owned(),
        };
        for action in &mut settings.actions {
            if action.kind == ActionKind::Translate {
                action.provider_id = Some("custom".to_owned());
                action.model_id = Some("deleted-model".to_owned());
            } else if action.kind == ActionKind::Explain {
                action.provider_id = Some("deleted-provider".to_owned());
                action.model_id = Some("some-model".to_owned());
            }
        }
        fs::write(&path, serde_json::to_string_pretty(&settings).unwrap()).unwrap();

        let loaded = load_settings(&path).expect("Orphan references should be healed on load");
        assert_eq!(
            loaded.settings.default_ai,
            crate::models::DefaultAiModel::default()
        );
        let translate = loaded
            .settings
            .actions
            .iter()
            .find(|a| a.kind == ActionKind::Translate)
            .unwrap();
        assert_eq!(translate.model_id.as_deref(), Some(""));
        let explain = loaded
            .settings
            .actions
            .iter()
            .find(|a| a.kind == ActionKind::Explain)
            .unwrap();
        assert_eq!(explain.provider_id.as_deref(), Some(""));
        assert_eq!(explain.model_id.as_deref(), Some(""));
    }
}
