use anyhow::{Context, Result, bail};
use camino::Utf8Path;
use serde::{Deserialize, Deserializer, Serialize};

use crate::path::WorkspacePath;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct RepositoryConfig {
    pub name: String,
    #[serde(default)]
    pub serve: ServeSettings,
    #[serde(default)]
    pub policy: Vec<Policy>,
    #[serde(default)]
    pub plugin: Vec<PluginConfig>,
    #[serde(default)]
    pub task: Vec<TaskConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServeSettings {
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_plugin_url_prefix")]
    pub plugin_url_prefix: String,
    #[serde(default = "default_policy_url_prefix")]
    pub policy_url_prefix: String,
    #[serde(default = "default_info_url_prefix")]
    pub info_url_prefix: String,
}

impl Default for ServeSettings {
    fn default() -> Self {
        Self {
            port: default_port(),
            plugin_url_prefix: default_plugin_url_prefix(),
            policy_url_prefix: default_policy_url_prefix(),
            info_url_prefix: default_info_url_prefix(),
        }
    }
}

fn default_port() -> u16 {
    3000
}

fn default_plugin_url_prefix() -> String {
    "/.plugin".into()
}

fn default_policy_url_prefix() -> String {
    "/.policy".into()
}

fn default_info_url_prefix() -> String {
    "/.info".into()
}

fn default_policy_get() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
pub struct Policy {
    #[serde(deserialize_with = "deserialize_workspace_path")]
    pub path: WorkspacePath,
    #[serde(flatten)]
    pub permissions: PolicyPermissions,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct PolicyPermissions {
    #[serde(rename = "GET", default = "default_policy_get")]
    pub get: bool,
    #[serde(rename = "POST", default)]
    pub post: bool,
    #[serde(rename = "PUT", default)]
    pub put: bool,
    #[serde(rename = "DELETE", default)]
    pub delete: bool,
}

impl PolicyPermissions {
    pub fn deny_all() -> Self {
        Self {
            get: false,
            post: false,
            put: false,
            delete: false,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PluginConfig {
    pub name: String,
    pub runner: String,
    pub command: Vec<String>,
    pub trigger: String,
    #[serde(default, deserialize_with = "deserialize_optional_workspace_path")]
    pub path: Option<WorkspacePath>,
    #[serde(default)]
    pub deps: Vec<String>,
    // URL prefix なので、 `WorkspacePath` ではなく文字列で受け取る。検証は後で行う。
    pub mount: Option<String>,
    #[serde(flatten)]
    pub extra: std::collections::BTreeMap<String, toml::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TaskConfig {
    pub name: String,
    pub steps: Vec<String>,
}

impl RepositoryConfig {
    pub fn load_toml(text: &str) -> Result<Self> {
        toml::from_str(text).context("failed to parse .repo/config.toml")
    }

    pub fn load(repository_root: &Utf8Path) -> Result<Self> {
        let config_path = repository_root.join(".repo").join("config.toml");
        if !config_path.is_file() {
            bail!("missing .repo/config.toml");
        }
        let config_text = std::fs::read_to_string(config_path.as_std_path())
            .context("failed to read .repo/config.toml")?;
        let mut config = Self::load_toml(&config_text)?;
        config.insert_implicit_mount_policies()?;
        config.validate(repository_root)?;
        Ok(config)
    }

    fn insert_implicit_mount_policies(&mut self) -> Result<()> {
        let mut implicit_policies = Vec::new();
        for plugin in &self.plugin {
            let Some(mount) = &plugin.mount else {
                continue;
            };
            implicit_policies.push(Policy {
                path: WorkspacePath::from_path_str(mount.trim_start_matches('/'))?,
                permissions: PolicyPermissions {
                    get: true,
                    post: false,
                    put: false,
                    delete: false,
                },
            });
        }

        if implicit_policies.is_empty() {
            return Ok(());
        }

        implicit_policies.append(&mut self.policy);
        self.policy = implicit_policies;
        Ok(())
    }

    fn validate(&self, repository_root: &Utf8Path) -> Result<()> {
        if self.name.is_empty() {
            bail!("name must not be empty");
        }
        validate_url_prefix("serve.plugin_url_prefix", &self.serve.plugin_url_prefix)?;
        validate_url_prefix("serve.policy_url_prefix", &self.serve.policy_url_prefix)?;
        validate_url_prefix("serve.info_url_prefix", &self.serve.info_url_prefix)?;

        let prefixes = [
            ("serve.plugin_url_prefix", self.serve.plugin_url_prefix.as_str()),
            ("serve.policy_url_prefix", self.serve.policy_url_prefix.as_str()),
            ("serve.info_url_prefix", self.serve.info_url_prefix.as_str()),
        ];
        let reserved_url_prefix_paths = reserved_url_prefix_paths(&self.serve)?;
        for index in 0..prefixes.len() {
            for other_index in (index + 1)..prefixes.len() {
                if prefixes[index].1 == prefixes[other_index].1 {
                    bail!(
                        "{} and {} must be different",
                        prefixes[index].0,
                        prefixes[other_index].0
                    );
                }
            }
        }

        for policy in &self.policy {
            if contains_glob_metachar(policy.path.as_str()) {
                bail!("policy path must not use glob syntax");
            }
            if policy.path.is_reserved() {
                bail!("policy path must not target .repo/");
            }
            if path_uses_reserved_url_prefix(&policy.path, &reserved_url_prefix_paths) {
                bail!("policy path must not target reserved url prefix");
            }
        }

        for plugin in &self.plugin {
            if !is_valid_plugin_name(&plugin.name) {
                bail!(
                    "plugin name must match [A-Za-z_][A-Za-z0-9_-]*: {}",
                    plugin.name
                );
            }
            if plugin.runner != "command" {
                bail!("unsupported plugin runner: {}", plugin.runner);
            }
            if plugin.command.is_empty() {
                bail!("plugin command must not be empty");
            }
            if let Some(path) = &plugin.path {
                if contains_glob_metachar(path.as_str()) {
                    bail!("plugin path must not use glob syntax");
                }
                if path.is_reserved() {
                    bail!("plugin path must not target .repo/");
                }
                if path_uses_reserved_url_prefix(path, &reserved_url_prefix_paths) {
                    bail!("plugin path must not target reserved url prefix");
                }
            }
            for dependency in &plugin.deps {
                if !is_valid_plugin_name(dependency) {
                    bail!(
                        "plugin dependency name must match [A-Za-z_][A-Za-z0-9_-]*: {}",
                        dependency
                    );
                }
                if dependency == &plugin.name {
                    bail!("plugin must not depend on itself: {}", plugin.name);
                }
                if self.find_plugin(dependency).is_none() {
                    bail!(
                        "plugin dependency not found: {} -> {}",
                        plugin.name,
                        dependency
                    );
                }
            }
            if plugin.trigger == "manual" {
                if plugin.path.is_some() {
                    bail!("manual plugin must not set path");
                }
            } else if plugin.path.is_none() {
                bail!("non-manual plugin must set path");
            }
            if let Some(mount) = &plugin.mount {
                if !mount.starts_with('/') || !mount.ends_with('/') {
                    bail!("plugin mount must start and end with /");
                }

                let relative = WorkspacePath::from_path_str(mount.trim_start_matches('/'))
                    .expect("validated mount path should parse");
                if path_uses_reserved_url_prefix(&relative, &reserved_url_prefix_paths) {
                    bail!("plugin mount must not target reserved url prefix");
                }
                if relative.as_str() != "." {
                    let target = relative.join_to(repository_root);
                    if target.is_dir() {
                        bail!(
                            "plugin mount conflicts with repository directory: {}",
                            mount
                        );
                    }
                }

                if self
                    .plugin
                    .iter()
                    .filter_map(|candidate| candidate.mount.as_ref())
                    .filter(|candidate| *candidate == mount)
                    .count()
                    > 1
                {
                    bail!("duplicate plugin mount: {}", mount);
                }
            }
        }

        Ok(())
    }

    pub fn find_plugin(&self, name: &str) -> Option<&PluginConfig> {
        self.plugin.iter().find(|plugin| plugin.name == name)
    }

    pub fn find_task(&self, name: &str) -> Option<&TaskConfig> {
        self.task.iter().find(|task| task.name == name)
    }
}

fn deserialize_workspace_path<'de, D>(deserializer: D) -> std::result::Result<WorkspacePath, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    WorkspacePath::from_path_str(&value).map_err(serde::de::Error::custom)
}

fn deserialize_optional_workspace_path<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<WorkspacePath>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    value
        .map(|raw| WorkspacePath::from_path_str(&raw).map_err(serde::de::Error::custom))
        .transpose()
}

fn contains_glob_metachar(value: &str) -> bool {
    value.contains('*') || value.contains('?') || value.contains('[')
}

fn validate_url_prefix(name: &str, value: &str) -> Result<()> {
    if !value.starts_with('/') || value.trim_matches('/').is_empty() || value.ends_with('/') {
        bail!("{name} must start with /, must not be empty, and must not end with /");
    }
    Ok(())
}

fn reserved_url_prefix_paths(settings: &ServeSettings) -> Result<Vec<WorkspacePath>> {
    [
        settings.plugin_url_prefix.as_str(),
        settings.policy_url_prefix.as_str(),
        settings.info_url_prefix.as_str(),
    ]
    .into_iter()
    .map(|prefix| WorkspacePath::from_path_str(prefix.trim_start_matches('/')))
    .collect()
}

fn path_uses_reserved_url_prefix(path: &WorkspacePath, reserved_prefixes: &[WorkspacePath]) -> bool {
    reserved_prefixes
        .iter()
        .any(|reserved_prefix| path.starts_with(reserved_prefix))
}

fn is_valid_plugin_name(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    if !first.is_ascii_alphabetic() && first != '_' {
        return false;
    }

    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_rule_defaults_get_only() {
        let rule: Policy = toml::from_str(
            r#"
path = "docs/"
"#,
        )
        .unwrap();

        assert_eq!(rule.path.as_str(), "docs");
        assert!(rule.path.is_directory());
        assert!(rule.permissions.get);
        assert!(!rule.permissions.post);
        assert!(!rule.permissions.put);
        assert!(!rule.permissions.delete);
    }

    #[test]
    fn policy_rule_requires_path() {
        let error = toml::from_str::<Policy>(
            r#"
GET = true
"#,
        )
        .unwrap_err();

        assert!(error.to_string().contains("missing field `path`"));
    }

    #[test]
    fn serve_settings_defaults_prefixes() {
        let settings: ServeSettings = toml::from_str("").unwrap();

        assert_eq!(settings.port, 3000);
        assert_eq!(settings.plugin_url_prefix, "/.plugin");
        assert_eq!(settings.policy_url_prefix, "/.policy");
        assert_eq!(settings.info_url_prefix, "/.info");
    }

    #[test]
    fn repository_config_rejects_prefix_without_leading_slash() {
        let config = RepositoryConfig {
            name: "repo".into(),
            serve: ServeSettings {
                port: 3000,
                plugin_url_prefix: ".plugin".into(),
                policy_url_prefix: "/.policy".into(),
                info_url_prefix: "/.info".into(),
            },
            policy: Vec::new(),
            plugin: Vec::new(),
            task: Vec::new(),
        };

        let error = config.validate(Utf8Path::new(".")).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("serve.plugin_url_prefix must start with /")
        );
    }

    #[test]
    fn repository_config_rejects_prefix_with_trailing_slash() {
        let config = RepositoryConfig {
            name: "repo".into(),
            serve: ServeSettings {
                port: 3000,
                plugin_url_prefix: "/.plugin/".into(),
                policy_url_prefix: "/.policy/".into(),
                info_url_prefix: "/.info".into(),
            },
            policy: Vec::new(),
            plugin: Vec::new(),
            task: Vec::new(),
        };

        let error = config.validate(Utf8Path::new(".")).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("serve.plugin_url_prefix must start with /, must not be empty, and must not end with /")
        );
    }

    #[test]
    fn repository_config_rejects_duplicate_virtual_prefixes() {
        let config = RepositoryConfig {
            name: "repo".into(),
            serve: ServeSettings {
                port: 3000,
                plugin_url_prefix: "/.plugin".into(),
                policy_url_prefix: "/.policy".into(),
                info_url_prefix: "/.policy".into(),
            },
            policy: Vec::new(),
            plugin: Vec::new(),
            task: Vec::new(),
        };

        let error = config.validate(Utf8Path::new(".")).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("serve.policy_url_prefix and serve.info_url_prefix must be different")
        );
    }

    #[test]
    fn repository_config_requires_name() {
        let error = RepositoryConfig::load_toml(
            r#"
[serve]
port = 3000
"#,
        )
        .unwrap_err();

        assert!(error.to_string().contains("failed to parse .repo/config.toml"));
    }

    #[test]
    fn repository_config_rejects_policy_path_with_leading_slash() {
        let error = toml::from_str::<Policy>(
            r#"
path = "/viewer/"
GET = true
"#,
        )
        .unwrap_err();

        assert!(error.to_string().contains("absolute paths are not allowed"));
    }

    #[test]
    fn repository_config_rejects_policy_path_with_glob_syntax() {
        let config = RepositoryConfig {
            name: "repo".into(),
            serve: ServeSettings::default(),
            policy: vec![Policy {
                path: WorkspacePath::from_path_str("viewer/**").unwrap(),
                permissions: PolicyPermissions {
                    get: true,
                    post: false,
                    put: false,
                    delete: false,
                },
            }],
            plugin: Vec::new(),
            task: Vec::new(),
        };

        let error = config.validate(Utf8Path::new(".")).unwrap_err();

        assert!(error.to_string().contains("policy path must not use glob syntax"));
    }

    #[test]
    fn mount_inserts_implicit_get_only_policy_before_explicit_rules() {
        let mut config = RepositoryConfig {
            name: "repo".into(),
            serve: ServeSettings::default(),
            policy: vec![Policy {
                path: WorkspacePath::from_path_str("assets/").unwrap(),
                permissions: PolicyPermissions::deny_all(),
            }],
            plugin: vec![PluginConfig {
                name: "assets".into(),
                runner: "command".into(),
                command: vec!["echo".into()],
                trigger: "manual".into(),
                path: None,
                deps: Vec::new(),
                mount: Some("/assets/".into()),
                extra: Default::default(),
            }],
            task: Vec::new(),
        };

        config.insert_implicit_mount_policies().unwrap();

        assert_eq!(config.policy.len(), 2);
        assert_eq!(config.policy[0].path.as_str(), "assets");
        assert!(config.policy[0].permissions.get);
        assert!(!config.policy[0].permissions.post);
        assert_eq!(config.policy[1].path.as_str(), "assets");
        assert!(!config.policy[1].permissions.get);
    }

    #[test]
    fn repository_config_rejects_invalid_plugin_name() {
        let config = RepositoryConfig {
            name: "repo".into(),
            serve: ServeSettings::default(),
            policy: Vec::new(),
            plugin: vec![PluginConfig {
                name: "bad.name".into(),
                runner: "command".into(),
                command: vec!["echo".into()],
                trigger: "manual".into(),
                path: None,
                deps: Vec::new(),
                mount: None,
                extra: Default::default(),
            }],
            task: Vec::new(),
        };

        let error = config.validate(Utf8Path::new(".")).unwrap_err();

        assert!(error.to_string().contains("plugin name must match"));
    }

    #[test]
    fn repository_config_rejects_missing_plugin_dependency() {
        let config = RepositoryConfig {
            name: "repo".into(),
            serve: ServeSettings::default(),
            policy: Vec::new(),
            plugin: vec![PluginConfig {
                name: "preview".into(),
                runner: "command".into(),
                command: vec!["echo".into()],
                trigger: "manual".into(),
                path: None,
                deps: vec!["build-wasm".into()],
                mount: None,
                extra: Default::default(),
            }],
            task: Vec::new(),
        };

        let error = config.validate(Utf8Path::new(".")).unwrap_err();

        assert!(error.to_string().contains("plugin dependency not found"));
    }

    #[test]
    fn load_toml_preserves_plugin_specific_settings() {
        let config = RepositoryConfig::load_toml(
            r#"
name = "repo"

[[plugin]]
name = "build-md-preview"
runner = "command"
command = ["node", "./plugins/md_preview/build.mjs"]
trigger = "manual"

[plugin.md_preview]
enabled = true

[[plugin.md_preview.enhance]]
name = "embedded-models"
url = "{MOUNT_BUILD_WASM}enhance.js"
entrypoint = "default"
"#,
        )
        .unwrap();

        let plugin = &config.plugin[0];
        let md_preview = plugin.extra.get("md_preview").unwrap().as_table().unwrap();
        assert_eq!(md_preview.get("enabled").unwrap().as_bool(), Some(true));

        let enhancers = md_preview.get("enhance").unwrap().as_array().unwrap();
        let enhancer = enhancers[0].as_table().unwrap();
        assert_eq!(
            enhancer.get("name").unwrap().as_str(),
            Some("embedded-models")
        );
        assert_eq!(
            enhancer.get("entrypoint").unwrap().as_str(),
            Some("default")
        );
    }

    #[test]
    fn repository_config_rejects_policy_path_under_reserved_url_prefix() {
        let config = RepositoryConfig {
            name: "repo".into(),
            serve: ServeSettings::default(),
            policy: vec![Policy {
                path: WorkspacePath::from_path_str(".info/cache.txt").unwrap(),
                permissions: PolicyPermissions {
                    get: true,
                    post: false,
                    put: false,
                    delete: false,
                },
            }],
            plugin: Vec::new(),
            task: Vec::new(),
        };

        let error = config.validate(Utf8Path::new(".")).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("policy path must not target reserved url prefix")
        );
    }

    #[test]
    fn repository_config_requires_path_for_non_manual_plugin() {
        let config = RepositoryConfig {
            name: "repo".into(),
            serve: ServeSettings::default(),
            policy: Vec::new(),
            plugin: vec![PluginConfig {
                name: "preview".into(),
                runner: "command".into(),
                command: vec!["echo".into()],
                trigger: "GET".into(),
                path: None,
                deps: Vec::new(),
                mount: None,
                extra: Default::default(),
            }],
            task: Vec::new(),
        };

        let error = config.validate(Utf8Path::new(".")).unwrap_err();

        assert!(error.to_string().contains("non-manual plugin must set path"));
    }

    #[test]
    fn repository_config_rejects_manual_plugin_path() {
        let config = RepositoryConfig {
            name: "repo".into(),
            serve: ServeSettings::default(),
            policy: Vec::new(),
            plugin: vec![PluginConfig {
                name: "preview".into(),
                runner: "command".into(),
                command: vec!["echo".into()],
                trigger: "manual".into(),
                path: Some(WorkspacePath::from_path_str("docs/").unwrap()),
                deps: Vec::new(),
                mount: None,
                extra: Default::default(),
            }],
            task: Vec::new(),
        };

        let error = config.validate(Utf8Path::new(".")).unwrap_err();

        assert!(error.to_string().contains("manual plugin must not set path"));
    }
}
