use anyhow::{Context, Result, bail};
use camino::Utf8Path;
use serde::{Deserialize, Deserializer, Serialize};

use crate::domain::workspace_path::WorkspacePath;

#[derive(Debug, Clone, Deserialize, Default)]
pub(crate) struct RepositoryConfig {
    pub name: String,
    #[serde(default)]
    pub serve: ServeSettings,
    #[serde(default)]
    pub policy: Vec<Policy>,
    #[serde(default)]
    pub ignore: IgnoreConfig,
    #[serde(default)]
    pub plugin: Vec<PluginConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ServeSettings {
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

#[derive(Debug, Clone, Default)]
pub(crate) struct ServeSettingsOverride {
    pub port: Option<u16>,
    pub plugin_url_prefix: Option<String>,
    pub policy_url_prefix: Option<String>,
    pub info_url_prefix: Option<String>,
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

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Policy {
    #[serde(deserialize_with = "deserialize_workspace_path")]
    pub path: WorkspacePath,
    #[serde(flatten)]
    pub permissions: PolicyPermissions,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub(crate) struct IgnoreConfig {
    #[serde(default, deserialize_with = "deserialize_workspace_paths")]
    pub paths: Vec<WorkspacePath>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct PolicyPermissions {
    #[serde(rename = "GET", default)]
    pub get: Vec<String>,
    #[serde(rename = "POST", default)]
    pub post: Vec<String>,
    #[serde(rename = "PUT", default)]
    pub put: Vec<String>,
    #[serde(rename = "DELETE", default)]
    pub delete: Vec<String>,
}

impl PolicyPermissions {
    pub fn deny_all() -> Self {
        Self {
            get: Vec::new(),
            post: Vec::new(),
            put: Vec::new(),
            delete: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct PluginConfig {
    pub name: String,
    pub runner: String,
    #[serde(default)]
    pub command: Vec<String>,
    #[serde(default)]
    pub allow: Vec<String>,
    // URL prefix なので、 `WorkspacePath` ではなく文字列で受け取る。検証は後で行う。
    pub mount: Option<String>,
    #[serde(flatten)]
    pub extra: std::collections::BTreeMap<String, toml::Value>,
}

impl RepositoryConfig {
    pub fn load_toml(text: &str) -> Result<Self> {
        toml::from_str(text).context("failed to parse .repo/config.toml")
    }

    pub fn load_with_serve_overrides(
        repository_root: &Utf8Path,
        overrides: &ServeSettingsOverride,
    ) -> Result<Self> {
        let config_path = repository_root.join(".repo").join("config.toml");
        if !config_path.is_file() {
            bail!("missing .repo/config.toml");
        }
        let config_text = std::fs::read_to_string(config_path.as_std_path())
            .context("failed to read .repo/config.toml")?;
        let mut config = Self::load_toml(&config_text)?;
        config.apply_serve_overrides(overrides);
        config.insert_implicit_mount_policies()?;
        config.validate(repository_root)?;
        Ok(config)
    }

    fn apply_serve_overrides(&mut self, overrides: &ServeSettingsOverride) {
        if let Some(port) = overrides.port {
            self.serve.port = port;
        }
        if let Some(plugin_url_prefix) = &overrides.plugin_url_prefix {
            self.serve.plugin_url_prefix = plugin_url_prefix.clone();
        }
        if let Some(policy_url_prefix) = &overrides.policy_url_prefix {
            self.serve.policy_url_prefix = policy_url_prefix.clone();
        }
        if let Some(info_url_prefix) = &overrides.info_url_prefix {
            self.serve.info_url_prefix = info_url_prefix.clone();
        }
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
                    get: plugin.allow.clone(),
                    post: Vec::new(),
                    put: Vec::new(),
                    delete: Vec::new(),
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
            (
                "serve.plugin_url_prefix",
                self.serve.plugin_url_prefix.as_str(),
            ),
            (
                "serve.policy_url_prefix",
                self.serve.policy_url_prefix.as_str(),
            ),
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

        for path in &self.ignore.paths {
            if path.as_str() == "." {
                bail!("ignore path must not target repository root");
            }
            if contains_glob_metachar(path.as_str()) {
                bail!("ignore path must not use glob syntax");
            }
            if path.is_reserved() {
                bail!("ignore path must not target .repo/");
            }
            if path_uses_reserved_url_prefix(path, &reserved_url_prefix_paths) {
                bail!("ignore path must not target reserved url prefix");
            }
        }

        for plugin in &self.plugin {
            if !is_valid_plugin_name(&plugin.name) {
                bail!(
                    "plugin name must match [A-Za-z_][A-Za-z0-9_-]*: {}",
                    plugin.name
                );
            }
            if plugin.runner != "command" && plugin.runner != "default" {
                bail!("unsupported plugin runner: {}", plugin.runner);
            }
            if plugin.runner == "command" && plugin.command.is_empty() {
                bail!("plugin command must not be empty");
            }
            if plugin.runner == "default" && !plugin.command.is_empty() {
                bail!("default plugin must not set command: {}", plugin.name);
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
}

fn deserialize_workspace_path<'de, D>(
    deserializer: D,
) -> std::result::Result<WorkspacePath, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    WorkspacePath::from_path_str(&value).map_err(serde::de::Error::custom)
}

fn deserialize_workspace_paths<'de, D>(
    deserializer: D,
) -> std::result::Result<Vec<WorkspacePath>, D::Error>
where
    D: Deserializer<'de>,
{
    let values = Vec::<String>::deserialize(deserializer)?;
    values
        .into_iter()
        .map(|value| WorkspacePath::from_path_str(&value).map_err(serde::de::Error::custom))
        .collect()
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

fn path_uses_reserved_url_prefix(
    path: &WorkspacePath,
    reserved_prefixes: &[WorkspacePath],
) -> bool {
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
    fn policy_rule_defaults_deny_all() {
        let rule: Policy = toml::from_str(
            r#"
path = "docs/"
"#,
        )
        .unwrap();

        assert_eq!(rule.path.as_str(), "docs");
        assert!(rule.path.is_directory());
        assert!(rule.permissions.get.is_empty());
        assert!(rule.permissions.post.is_empty());
        assert!(rule.permissions.put.is_empty());
        assert!(rule.permissions.delete.is_empty());
    }

    #[test]
    fn policy_rule_requires_path() {
        let error = toml::from_str::<Policy>(
            r#"
GET = ["alice_browser"]
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
    fn load_with_serve_overrides_replaces_serve_settings() {
        let root = std::env::temp_dir().join(format!(
            "workspace-fs-config-override-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join(".repo")).unwrap();
        std::fs::write(
            root.join(".repo").join("config.toml"),
            r#"
name = "repo"

[serve]
port = 3030
plugin_url_prefix = "/.plugin"
policy_url_prefix = "/.policy"
info_url_prefix = "/.info"
"#,
        )
        .unwrap();

        let config = RepositoryConfig::load_with_serve_overrides(
            Utf8Path::from_path(root.as_path()).unwrap(),
            &ServeSettingsOverride {
                port: Some(4040),
                plugin_url_prefix: Some("/.plugin2".into()),
                policy_url_prefix: Some("/.policy2".into()),
                info_url_prefix: Some("/.info2".into()),
            },
        )
        .unwrap();

        assert_eq!(config.serve.port, 4040);
        assert_eq!(config.serve.plugin_url_prefix, "/.plugin2");
        assert_eq!(config.serve.policy_url_prefix, "/.policy2");
        assert_eq!(config.serve.info_url_prefix, "/.info2");

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn repository_config_loads_ignore_paths() {
        let config = RepositoryConfig::load_toml(
            r#"
name = "repo"

[ignore]
paths = [
  ".git",
  "LICENSE",
]
"#,
        )
        .unwrap();

        let paths = config
            .ignore
            .paths
            .iter()
            .map(WorkspacePath::as_str)
            .collect::<Vec<_>>();
        assert_eq!(paths, vec![".git", "LICENSE"]);
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
            ignore: IgnoreConfig::default(),
            plugin: Vec::new(),
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
            ignore: IgnoreConfig::default(),
            plugin: Vec::new(),
        };

        let error = config.validate(Utf8Path::new(".")).unwrap_err();

        assert!(error.to_string().contains(
            "serve.plugin_url_prefix must start with /, must not be empty, and must not end with /"
        ));
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
            ignore: IgnoreConfig::default(),
            plugin: Vec::new(),
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

        assert!(
            error
                .to_string()
                .contains("failed to parse .repo/config.toml")
        );
    }

    #[test]
    fn repository_config_rejects_policy_path_with_leading_slash() {
        let error = toml::from_str::<Policy>(
            r#"
path = "/viewer/"
GET = ["alice_browser"]
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
                    get: vec!["alice_browser".into()],
                    post: Vec::new(),
                    put: Vec::new(),
                    delete: Vec::new(),
                },
            }],
            ignore: IgnoreConfig::default(),
            plugin: Vec::new(),
        };

        let error = config.validate(Utf8Path::new(".")).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("policy path must not use glob syntax")
        );
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
            ignore: IgnoreConfig::default(),
            plugin: vec![PluginConfig {
                name: "assets".into(),
                runner: "command".into(),
                command: vec!["echo".into()],
                allow: vec!["alice_browser".into()],
                mount: Some("/assets/".into()),
                extra: Default::default(),
            }],
        };

        config.insert_implicit_mount_policies().unwrap();

        assert_eq!(config.policy.len(), 2);
        assert_eq!(config.policy[0].path.as_str(), "assets");
        assert_eq!(config.policy[0].permissions.get, vec!["alice_browser"]);
        assert!(config.policy[0].permissions.post.is_empty());
        assert_eq!(config.policy[1].path.as_str(), "assets");
        assert!(config.policy[1].permissions.get.is_empty());
    }

    #[test]
    fn repository_config_rejects_invalid_plugin_name() {
        let config = RepositoryConfig {
            name: "repo".into(),
            serve: ServeSettings::default(),
            policy: Vec::new(),
            ignore: IgnoreConfig::default(),
            plugin: vec![PluginConfig {
                name: "bad.name".into(),
                runner: "command".into(),
                command: vec!["echo".into()],
                allow: vec!["alice_browser".into()],
                mount: None,
                extra: Default::default(),
            }],
        };

        let error = config.validate(Utf8Path::new(".")).unwrap_err();

        assert!(error.to_string().contains("plugin name must match"));
    }

    #[test]
    fn repository_config_accepts_plugin_without_dependencies() {
        let config = RepositoryConfig {
            name: "repo".into(),
            serve: ServeSettings::default(),
            policy: Vec::new(),
            ignore: IgnoreConfig::default(),
            plugin: vec![PluginConfig {
                name: "preview".into(),
                runner: "command".into(),
                command: vec!["echo".into()],
                allow: vec!["alice_browser".into()],
                mount: None,
                extra: Default::default(),
            }],
        };

        config.validate(Utf8Path::new(".")).unwrap();
    }

    #[test]
    fn repository_config_accepts_default_plugin_without_command() {
        let config = RepositoryConfig {
            name: "repo".into(),
            serve: ServeSettings::default(),
            policy: Vec::new(),
            ignore: IgnoreConfig::default(),
            plugin: vec![PluginConfig {
                name: "preview".into(),
                runner: "default".into(),
                command: Vec::new(),
                allow: vec!["alice_browser".into()],
                mount: None,
                extra: Default::default(),
            }],
        };

        config.validate(Utf8Path::new(".")).unwrap();
    }

    #[test]
    fn repository_config_rejects_default_plugin_command_override() {
        let config = RepositoryConfig {
            name: "repo".into(),
            serve: ServeSettings::default(),
            policy: Vec::new(),
            ignore: IgnoreConfig::default(),
            plugin: vec![PluginConfig {
                name: "preview".into(),
                runner: "default".into(),
                command: vec!["echo".into()],
                allow: vec!["alice_browser".into()],
                mount: None,
                extra: Default::default(),
            }],
        };

        let error = config.validate(Utf8Path::new(".")).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("default plugin must not set command")
        );
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
allow = ["alice_browser"]

[plugin.md_preview]
enabled = true

[[plugin.md_preview.transform]]
name = "katex"
url = "{MOUNT_MD_PREVIEW}katex_transform.js"
entrypoint = "default"

[plugin.md_preview.md_viewer]
additional_js = ["assets/header.js"]
"#,
        )
        .unwrap();

        let plugin = &config.plugin[0];
        let md_preview = plugin.extra.get("md_preview").unwrap().as_table().unwrap();
        assert_eq!(md_preview.get("enabled").unwrap().as_bool(), Some(true));

        let transforms = md_preview.get("transform").unwrap().as_array().unwrap();
        let transform = transforms[0].as_table().unwrap();
        assert_eq!(transform.get("name").unwrap().as_str(), Some("katex"));
        assert_eq!(
            transform.get("entrypoint").unwrap().as_str(),
            Some("default")
        );

        assert_eq!(
            md_preview
                .get("md_viewer")
                .unwrap()
                .as_table()
                .unwrap()
                .get("additional_js")
                .unwrap()
                .as_array()
                .unwrap()[0]
                .as_str(),
            Some("assets/header.js")
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
                    get: vec!["alice_browser".into()],
                    post: Vec::new(),
                    put: Vec::new(),
                    delete: Vec::new(),
                },
            }],
            ignore: IgnoreConfig::default(),
            plugin: Vec::new(),
        };

        let error = config.validate(Utf8Path::new(".")).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("policy path must not target reserved url prefix")
        );
    }

    #[test]
    fn repository_config_accepts_command_plugin_without_extra_metadata() {
        let config = RepositoryConfig {
            name: "repo".into(),
            serve: ServeSettings::default(),
            policy: Vec::new(),
            ignore: IgnoreConfig::default(),
            plugin: vec![PluginConfig {
                name: "preview".into(),
                runner: "command".into(),
                command: vec!["echo".into()],
                allow: vec!["alice_browser".into()],
                mount: None,
                extra: Default::default(),
            }],
        };

        config.validate(Utf8Path::new(".")).unwrap();
    }
}
