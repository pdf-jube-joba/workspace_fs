use std::process::Stdio;

use anyhow::{Context, Result, bail};
use camino::Utf8PathBuf;
use serde::Deserialize;
use serde_json::Value as JsonValue;
use tokio::process::Command;

use crate::{
    config::{PluginConfig, RepositoryConfig},
    identity::UserIdentity,
};

#[derive(Debug, Clone)]
pub struct PluginContext {
    pub repository_root: Utf8PathBuf,
    pub repository_name: String,
    pub plugin_name: String,
    pub output_directory: Utf8PathBuf,
    pub cache_directory: Utf8PathBuf,
    pub mount_url: Option<String>,
    pub user_identity: UserIdentity,
}

pub struct PluginRunner<'a> {
    repository_root: &'a camino::Utf8Path,
    repository_name: &'a str,
    config: &'a RepositoryConfig,
}

impl<'a> PluginRunner<'a> {
    pub fn new(
        repository_root: &'a camino::Utf8Path,
        repository_name: &'a str,
        config: &'a RepositoryConfig,
    ) -> Self {
        Self {
            repository_root,
            repository_name,
            config,
        }
    }

    pub async fn run_plugin(&self, plugin_name: &str, user_identity: &UserIdentity) -> Result<()> {
        let plugin = self
            .config
            .find_plugin(plugin_name)
            .with_context(|| format!("plugin not found: {plugin_name}"))?;
        self.invoke_plugin(plugin, user_identity).await
    }

    async fn invoke_plugin(
        &self,
        plugin: &PluginConfig,
        user_identity: &UserIdentity,
    ) -> Result<()> {
        let context = PluginContext {
            repository_root: self.repository_root.to_owned(),
            repository_name: self.repository_name.to_owned(),
            plugin_name: plugin.name.clone(),
            output_directory: self
                .repository_root
                .join(".repo")
                .join(&plugin.name)
                .join("generated"),
            cache_directory: self
                .repository_root
                .join(".repo")
                .join(&plugin.name)
                .join("cache"),
            mount_url: plugin.mount.clone(),
            user_identity: user_identity.clone(),
        };

        tokio::fs::create_dir_all(context.output_directory.as_std_path())
            .await
            .context("failed to create plugin output directory")?;
        tokio::fs::create_dir_all(context.cache_directory.as_std_path())
            .await
            .context("failed to create plugin cache directory")?;

        let plugin_command = resolve_plugin_command(plugin)?;
        let program = expand_placeholder(&plugin_command[0], &context)?;
        let args = plugin_command[1..]
            .iter()
            .map(|arg| expand_placeholder(arg, &context))
            .collect::<Result<Vec<_>>>()?;
        let settings_json = resolved_plugin_settings_json(plugin, &context)?;

        tracing::info!(plugin = %plugin.name, "running plugin");

        let mut command = Command::new(&program);
        command
            .args(&args)
            .current_dir(context.repository_root.as_std_path())
            .env(
                "WORKSPACE_FS_REPOSITORY_ROOT",
                context.repository_root.as_str(),
            )
            .env("WORKSPACE_FS_REPOSITORY_NAME", &context.repository_name)
            .env("WORKSPACE_FS_PLUGIN_NAME", &context.plugin_name)
            .env(
                "WORKSPACE_FS_OUTPUT_DIRECTORY",
                context.output_directory.as_str(),
            )
            .env(
                "WORKSPACE_FS_CACHE_DIRECTORY",
                context.cache_directory.as_str(),
            )
            .env("WORKSPACE_FS_PLUGIN_SETTINGS_JSON", settings_json)
            .env("WORKSPACE_FS_USER_IDENTITY", context.user_identity.as_str())
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        if let Some(mount_url) = &context.mount_url {
            command.env("MOUNT_URL", mount_url);
        }

        let status = command
            .status()
            .await
            .with_context(|| format!("failed to run plugin: {}", plugin.name))?;

        if !status.success() {
            bail!("plugin failed: {}: {}", plugin.name, status);
        }

        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct DefaultPluginConfig {
    plugin: Vec<DefaultPluginDefinition>,
}

#[derive(Debug, Deserialize)]
struct DefaultPluginDefinition {
    name: String,
    command: Vec<String>,
}

fn resolve_plugin_command(plugin: &PluginConfig) -> Result<Vec<String>> {
    match plugin.runner.as_str() {
        "command" => Ok(plugin.command.clone()),
        "default" => resolve_default_plugin_command(&plugin.name),
        other => bail!("unsupported plugin runner: {other}"),
    }
}

fn resolve_default_plugin_command(plugin_name: &str) -> Result<Vec<String>> {
    let default_config_path = workspace_fs_root().join("default.toml");
    let text = std::fs::read_to_string(default_config_path.as_std_path())
        .context("failed to read _wfs/default.toml")?;
    let config: DefaultPluginConfig =
        toml::from_str(&text).context("failed to parse _wfs/default.toml")?;

    let definition = config
        .plugin
        .iter()
        .find(|definition| definition.name == plugin_name)
        .with_context(|| format!("default plugin not found: {plugin_name}"))?;
    if definition.command.is_empty() {
        bail!("default plugin command must not be empty: {plugin_name}");
    }
    Ok(definition.command.clone())
}

fn expand_placeholder(input: &str, context: &PluginContext) -> Result<String> {
    let workspace_fs_root = workspace_fs_root();
    let replacements = [
        ("{REPOSITORY_ROOT}", context.repository_root.as_str()),
        ("{REPOSITORY_NAME}", context.repository_name.as_str()),
        ("{PLUGIN_NAME}", context.plugin_name.as_str()),
        ("{OUTPOST_DIRECTORY}", context.output_directory.as_str()),
        ("{OUTPUT_DIRECTORY}", context.output_directory.as_str()),
        ("{WORKSPACE_FS_ROOT}", workspace_fs_root.as_str()),
    ];

    let mut value = input.to_owned();
    for (from, to) in replacements {
        value = value.replace(from, to);
    }
    let default_plugins_root = workspace_fs_root.join("default_plugins");
    value = value.replace("{DEFAULT_PLUGINS_ROOT}", default_plugins_root.as_str());
    if let Some(mount_url) = &context.mount_url {
        value = value.replace("{MOUNT_URL}", mount_url);
    }

    Ok(value)
}

fn workspace_fs_root() -> Utf8PathBuf {
    let manifest_dir = Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if manifest_dir.join("default.toml").is_file() {
        return manifest_dir;
    }
    if let Some(root) = manifest_dir.parent().and_then(|path| path.parent()) {
        let root = root.to_owned();
        if root.join("default.toml").is_file() {
            return root;
        }
    }
    manifest_dir
}

fn resolved_plugin_settings_json(plugin: &PluginConfig, context: &PluginContext) -> Result<String> {
    if plugin.extra.is_empty() {
        return Ok(String::from("{}"));
    }

    let mut object = serde_json::Map::new();
    for (key, value) in &plugin.extra {
        let value = plugin_setting_to_json(value, context)?;
        object.insert(key.clone(), value);
    }
    serde_json::to_string(&JsonValue::Object(object)).context("failed to serialize plugin settings")
}

fn plugin_setting_to_json(value: &toml::Value, context: &PluginContext) -> Result<JsonValue> {
    match value {
        toml::Value::String(text) => Ok(JsonValue::String(expand_placeholder(text, context)?)),
        toml::Value::Integer(value) => Ok(JsonValue::Number((*value).into())),
        toml::Value::Float(value) => serde_json::Number::from_f64(*value)
            .map(JsonValue::Number)
            .context("plugin setting float must be finite"),
        toml::Value::Boolean(value) => Ok(JsonValue::Bool(*value)),
        toml::Value::Datetime(value) => Ok(JsonValue::String(value.to_string())),
        toml::Value::Array(values) => Ok(JsonValue::Array(
            values
                .iter()
                .map(|item| plugin_setting_to_json(item, context))
                .collect::<Result<Vec<_>>>()?,
        )),
        toml::Value::Table(entries) => {
            let mut object = serde_json::Map::new();
            for (key, value) in entries {
                object.insert(key.clone(), plugin_setting_to_json(value, context)?);
            }
            Ok(JsonValue::Object(object))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn plugin_context() -> PluginContext {
        PluginContext {
            repository_root: Utf8PathBuf::from("/repo"),
            repository_name: "repo".into(),
            plugin_name: "plugin".into(),
            output_directory: Utf8PathBuf::from("/repo/.repo/plugin/generated"),
            cache_directory: Utf8PathBuf::from("/repo/.repo/plugin/cache"),
            mount_url: Some("/plugin-assets/".into()),
            user_identity: UserIdentity::new("user"),
        }
    }

    fn plugin_config_with_extra(extra: BTreeMap<String, toml::Value>) -> PluginConfig {
        PluginConfig {
            name: "plugin".into(),
            runner: "command".into(),
            command: vec!["echo".into()],
            allow: vec!["user".into()],
            _legacy_trigger: None,
            _legacy_path: None,
            _legacy_deps: Vec::new(),
            mount: Some("/plugin-assets/".into()),
            extra,
        }
    }

    #[test]
    fn resolve_default_plugin_command_reads_default_toml() {
        let command = resolve_default_plugin_command("md-preview").unwrap();

        assert_eq!(command[0], "node");
        assert!(
            command
                .iter()
                .any(|arg| arg.contains("md_preview/build.mjs"))
        );
    }

    #[test]
    fn expand_placeholder_replaces_common_values() {
        let value = expand_placeholder(
            "{REPOSITORY_ROOT}:{REPOSITORY_NAME}:{PLUGIN_NAME}:{OUTPUT_DIRECTORY}:{MOUNT_URL}",
            &plugin_context(),
        )
        .unwrap();

        assert_eq!(
            value,
            "/repo:repo:plugin:/repo/.repo/plugin/generated:/plugin-assets/"
        );
    }

    #[test]
    fn resolved_plugin_settings_json_expands_nested_placeholders() {
        let plugin = plugin_config_with_extra(BTreeMap::from([(
            "md_preview".into(),
            toml::Value::Table(toml::map::Map::from_iter([(
                "enhance".into(),
                toml::Value::Array(vec![toml::Value::Table(toml::map::Map::from_iter([
                    ("name".into(), toml::Value::String("embedded-models".into())),
                    (
                        "url".into(),
                        toml::Value::String("{MOUNT_URL}enhance.js".into()),
                    ),
                ]))]),
            )])),
        )]));

        let json = resolved_plugin_settings_json(&plugin, &plugin_context()).unwrap();

        assert_eq!(
            json,
            r#"{"md_preview":{"enhance":[{"name":"embedded-models","url":"/plugin-assets/enhance.js"}]}}"#
        );
    }
}
