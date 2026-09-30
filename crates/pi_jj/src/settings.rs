//! jj settings, loaded the way the jj CLI loads them, minus `--config` arguments.
//!
//! Layers, lowest first: jj-lib defaults, host/user names, user config files,
//! repo config, workspace config, then `JJ_*` environment overrides. Conditional
//! `[[--scope]]` tables are resolved last, as in the CLI.

use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use jj_lib::config::{ConfigLayer, ConfigResolutionContext, ConfigSource, StackedConfig};
use jj_lib::secure_config::SecureConfig;
use jj_lib::settings::UserSettings;
use jj_lib::workspace::{DefaultWorkspaceLoaderFactory, WorkspaceLoaderFactory as _};
use rand_chacha::ChaCha20Rng;
use rand_chacha::rand_core::SeedableRng as _;

/// Where configuration comes from. Tests use [`Env::isolated`] so a developer's
/// own jj config (signing, hooks) cannot leak into them.
#[derive(Clone, Debug, Default)]
pub struct Env {
    home_dir: Option<PathBuf>,
    /// The platform config directory, e.g. `~/.config`; jj uses `<it>/jj`.
    config_dir: Option<PathBuf>,
    /// `JJ_CONFIG`: replaces all user config paths when set.
    jj_config: Option<String>,
    hostname: String,
    username: String,
    vars: HashMap<String, String>,
}

impl Env {
    pub fn from_process() -> Self {
        let home_dir = etcetera::home_dir().ok();
        Self {
            config_dir: etcetera::choose_base_strategy()
                .ok()
                .map(|strategy| etcetera::BaseStrategy::config_dir(&strategy)),
            jj_config: env::var("JJ_CONFIG").ok(),
            hostname: whoami::hostname().unwrap_or_default(),
            username: whoami::username()
                .ok()
                .or_else(|| env::var("USER").ok())
                .unwrap_or_default(),
            vars: env::vars_os()
                .filter_map(|(key, value)| {
                    Some((key.into_string().ok()?, value.into_string().ok()?))
                })
                .collect(),
            home_dir,
        }
    }

    /// No user, repo or workspace config files, and no environment overrides.
    pub fn isolated(user_name: &str, user_email: &str) -> Self {
        let vars = HashMap::from([
            ("JJ_USER".to_owned(), user_name.to_owned()),
            ("JJ_EMAIL".to_owned(), user_email.to_owned()),
        ]);
        Self {
            hostname: "host".to_owned(),
            username: "user".to_owned(),
            vars,
            ..Self::default()
        }
    }

    /// User config files and directories that exist, in jj's order.
    fn user_config_paths(&self) -> Vec<PathBuf> {
        if let Some(paths) = &self.jj_config {
            return env::split_paths(paths)
                .filter(|path| !path.as_os_str().is_empty() && path.exists())
                .collect();
        }
        let platform_file = self
            .config_dir
            .as_ref()
            .map(|dir| dir.join("jj/config.toml"));
        let mut paths = Vec::new();
        if let Some(home) = &self.home_dir {
            let legacy = home.join(".jjconfig.toml");
            if legacy.exists() || platform_file.is_none() {
                paths.push(legacy);
            }
        }
        paths.extend(platform_file);
        paths.extend(self.config_dir.as_ref().map(|dir| dir.join("jj/conf.d")));
        paths.retain(|path| path.exists());
        paths
    }

    /// Repo and workspace config live under the user config directory, keyed by
    /// an ID stored in the repo, so a cloned repo cannot bring its own config.
    fn secure_config_path(&self, config: SecureConfig, kind: &str) -> Result<Option<PathBuf>> {
        let Some(dir) = &self.config_dir else {
            return Ok(None);
        };
        // Only used to name a new config ID when migrating a legacy repo config.
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64)
            ^ u64::from(std::process::id());
        let mut rng = ChaCha20Rng::seed_from_u64(seed);
        let loaded = config.maybe_load_config(&mut rng, &dir.join("jj").join(kind))?;
        Ok(loaded.config_file.filter(|path| path.exists()))
    }

    fn base_layer(&self) -> Result<ConfigLayer> {
        let mut layer = ConfigLayer::empty(ConfigSource::EnvBase);
        if !self.hostname.is_empty() {
            layer.set_value("operation.hostname", self.hostname.as_str())?;
        }
        if !self.username.is_empty() {
            layer.set_value("operation.username", self.username.as_str())?;
        }
        Ok(layer)
    }

    fn overrides_layer(&self) -> Result<ConfigLayer> {
        let mut layer = ConfigLayer::empty(ConfigSource::EnvOverrides);
        for (var, name) in [
            ("JJ_USER", "user.name"),
            ("JJ_EMAIL", "user.email"),
            ("JJ_TIMESTAMP", "debug.commit-timestamp"),
            ("JJ_OP_TIMESTAMP", "debug.operation-timestamp"),
            ("JJ_OP_HOSTNAME", "operation.hostname"),
            ("JJ_OP_USERNAME", "operation.username"),
        ] {
            if let Some(value) = self.vars.get(var) {
                layer.set_value(name, value.as_str())?;
            }
        }
        if let Some(Ok(seed)) = self
            .vars
            .get("JJ_RANDOMNESS_SEED")
            .map(|s| s.parse::<i64>())
        {
            layer.set_value("debug.randomness-seed", seed)?;
        }
        Ok(layer)
    }
}

/// Settings for the workspace at `workspace_root`, or user-level settings when
/// there is no workspace yet.
pub fn load(env: &Env, workspace_root: Option<&Path>) -> Result<UserSettings> {
    let mut config = StackedConfig::with_defaults();
    config.add_layer(env.base_layer()?);
    for path in env.user_config_paths() {
        if path.is_dir() {
            config.load_dir(ConfigSource::User, &path)?;
        } else {
            config.load_file(ConfigSource::User, path)?;
        }
    }

    let mut repo_path = None;
    if let Some(root) = workspace_root {
        let loader = DefaultWorkspaceLoaderFactory.create(root)?;
        repo_path = Some(loader.repo_path().to_owned());
        let repo = SecureConfig::new_repo(loader.repo_path().to_owned());
        if let Some(path) = env.secure_config_path(repo, "repos")? {
            config.load_file(ConfigSource::Repo, path)?;
        }
        let workspace = SecureConfig::new_workspace(root.join(".jj"));
        if let Some(path) = env.secure_config_path(workspace, "workspaces")? {
            config.load_file(ConfigSource::Workspace, path)?;
        }
    }
    config.add_layer(env.overrides_layer()?);

    let context = ConfigResolutionContext {
        home_dir: env.home_dir.as_deref(),
        repo_path: repo_path.as_deref(),
        workspace_path: workspace_root,
        command: None,
        hostname: &env.hostname,
        environment: &env.vars,
    };
    Ok(UserSettings::from_config(jj_lib::config::resolve(
        &config, &context,
    )?)?)
}
