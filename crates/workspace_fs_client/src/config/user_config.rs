use std::{borrow::Cow, collections::BTreeMap};

use anyhow::{Context, Result, anyhow, bail};
use camino::Utf8Path;
use serde::Deserialize;
use toml::Value;

#[derive(Debug, Clone, Deserialize)]
pub struct UserConfig {
    #[serde(default)]
    pub repository: Vec<UserRepositoryConfig>,
    #[serde(default)]
    pub task: Vec<UserTaskConfig>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RepositoryMode {
    Spawn,
    Attach,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UserServerConfig {
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(flatten)]
    pub values: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UserRepositoryConfig {
    pub name: String,
    pub mode: RepositoryMode,
    pub port: u16,
    #[serde(rename = "where")]
    pub where_: Option<String>,
    #[serde(rename = "as")]
    pub as_user: String,
    #[serde(default = "default_plugin_url_prefix")]
    pub plugin_url_prefix: String,
    #[serde(default, alias = "serve")]
    pub server: Option<UserServerConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UserTaskConfig {
    pub name: String,
    #[serde(default)]
    pub step: Vec<UserTaskStep>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UserTaskStep {
    pub repository: String,
    pub plugin: String,
}

impl UserConfig {
    pub fn load(repository_root: &Utf8Path) -> Result<Self> {
        let config_path = repository_root.join(".repo").join("user.toml");
        if !config_path.is_file() {
            bail!("missing .repo/user.toml");
        }
        let config_text = std::fs::read_to_string(config_path.as_std_path())
            .context("failed to read .repo/user.toml")?;
        let config = Self::load_toml(&config_text)?;
        config.validate()?;
        Ok(config)
    }

    pub fn load_toml(text: &str) -> Result<Self> {
        toml::from_str(text).context("failed to parse .repo/user.toml")
    }

    pub fn find_repository(&self, name: &str) -> Option<&UserRepositoryConfig> {
        self.repository
            .iter()
            .find(|repository| repository.name == name)
    }

    pub fn repositories_to_start(
        &self,
        selected_name: Option<&str>,
    ) -> Result<Vec<&UserRepositoryConfig>> {
        if let Some(name) = selected_name {
            return Ok(vec![self.find_repository(name).ok_or_else(|| {
                anyhow!("repository not found in .repo/user.toml: {name}")
            })?]);
        }

        if self.repository.is_empty() {
            bail!("no [[repository]] entries configured in .repo/user.toml");
        }

        Ok(self.repository.iter().collect())
    }

    pub fn find_task(&self, name: &str) -> Option<&UserTaskConfig> {
        self.task.iter().find(|task| task.name == name)
    }

    fn validate(&self) -> Result<()> {
        if self.repository.is_empty() {
            bail!("at least one [[repository]] entry is required in .repo/user.toml");
        }

        let mut seen_names = std::collections::BTreeSet::new();
        let mut seen_browser_ports = std::collections::BTreeSet::new();
        let mut seen_spawn_ports = std::collections::BTreeSet::new();

        for repository in &self.repository {
            if repository.name.trim().is_empty() {
                bail!("repository.name must not be empty");
            }
            if !seen_names.insert(repository.name.as_str()) {
                bail!(
                    "duplicate repository.name in .repo/user.toml: {}",
                    repository.name
                );
            }
            if repository.port == 0 {
                bail!("repository.port must not be zero: {}", repository.name);
            }
            if !seen_browser_ports.insert(repository.port) {
                bail!(
                    "duplicate repository.port in .repo/user.toml: {}",
                    repository.port
                );
            }
            if repository.as_user.trim().is_empty() {
                bail!("repository.as must not be empty");
            }

            match repository.mode {
                RepositoryMode::Spawn => {
                    if repository.where_.is_some() {
                        bail!(
                            "repository.where must not be set when mode = \"spawn\": {}",
                            repository.name
                        );
                    }
                    let server = repository.server.as_ref().ok_or_else(|| {
                        anyhow!(
                            "repository.server is required when mode = \"spawn\": {}",
                            repository.name
                        )
                    })?;
                    let port = server.port().ok_or_else(|| {
                        anyhow!(
                            "repository.server.port is required when mode = \"spawn\": {}",
                            repository.name
                        )
                    })?;
                    if port == 0 {
                        bail!(
                            "repository.server.port must not be zero: {}",
                            repository.name
                        );
                    }
                    if !seen_spawn_ports.insert(port) {
                        bail!("duplicate spawned server port in .repo/user.toml: {}", port);
                    }
                }
                RepositoryMode::Attach => {
                    let Some(where_) = &repository.where_ else {
                        bail!(
                            "repository.where is required when mode = \"attach\": {}",
                            repository.name
                        );
                    };
                    if where_.trim().is_empty() {
                        bail!("repository.where must not be empty");
                    }
                    if repository.server.is_some() {
                        bail!(
                            "repository.server is only valid when mode = \"spawn\": {}",
                            repository.name
                        );
                    }
                }
            }
        }

        let mut task_names = std::collections::BTreeSet::new();
        for task in &self.task {
            if task.name.trim().is_empty() {
                bail!("task.name must not be empty");
            }
            if !task_names.insert(task.name.as_str()) {
                bail!("duplicate task.name in .repo/user.toml: {}", task.name);
            }
            if task.step.is_empty() {
                bail!("task.step must not be empty: {}", task.name);
            }
            for step in &task.step {
                if self
                    .repository
                    .iter()
                    .all(|repository| repository.name != step.repository)
                {
                    bail!(
                        "task references unknown repository in .repo/user.toml: {} -> {}",
                        task.name,
                        step.repository
                    );
                }
                if step.plugin.trim().is_empty() {
                    bail!("task step plugin must not be empty: {}", task.name);
                }
            }
        }

        Ok(())
    }
}

impl UserRepositoryConfig {
    pub fn server_config(&self) -> Result<&UserServerConfig> {
        self.server
            .as_ref()
            .ok_or_else(|| anyhow!("repository.server is required for mode = \"spawn\""))
    }

    pub fn upstream_plugin_url_prefix(&self) -> Cow<'_, str> {
        match self.mode {
            RepositoryMode::Spawn | RepositoryMode::Attach => {
                Cow::Borrowed(&self.plugin_url_prefix)
            }
        }
    }
}

impl UserServerConfig {
    pub fn port(&self) -> Option<u16> {
        self.values
            .get("port")
            .and_then(Value::as_integer)
            .and_then(|value| u16::try_from(value).ok())
    }

    pub fn cli_args(&self) -> Result<Vec<String>> {
        let mut args = Vec::new();
        for (name, value) in &self.values {
            args.push(format!(
                "--{}={}",
                name.replace('_', "-"),
                scalar_value(name, value)?
            ));
        }
        args.extend(self.args.iter().cloned());
        Ok(args)
    }
}

fn default_plugin_url_prefix() -> String {
    "/.plugin".into()
}

fn scalar_value(name: &str, value: &Value) -> Result<String> {
    match value {
        Value::String(value) => Ok(value.clone()),
        Value::Integer(value) => Ok(value.to_string()),
        Value::Float(value) => Ok(value.to_string()),
        Value::Boolean(value) => Ok(value.to_string()),
        _ => bail!("repository.server.{name} must be a scalar value"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_toml_reads_user_config() {
        let config = UserConfig::load_toml(
            r#"
[[repository]]
name = "local"
mode = "spawn"
port = 3031
as = "alice"

[repository.server]
port = 3020
plugin_url_prefix = "/.plugin2"
"#,
        )
        .unwrap();

        assert_eq!(config.repository.len(), 1);
        assert_eq!(config.repository[0].name, "local");
        assert_eq!(config.repository[0].mode, RepositoryMode::Spawn);
        assert_eq!(config.repository[0].port, 3031);
        assert_eq!(config.repository[0].where_, None);
        assert_eq!(config.repository[0].as_user, "alice");
        assert_eq!(config.repository[0].plugin_url_prefix, "/.plugin");
        let server = config.repository[0].server.as_ref().unwrap();
        assert_eq!(server.port(), Some(3020));
        assert_eq!(
            server.cli_args().unwrap(),
            vec!["--plugin-url-prefix=/.plugin2", "--port=3020"]
        );
    }

    #[test]
    fn repositories_to_start_returns_all_when_name_is_omitted() {
        let config = UserConfig::load_toml(
            r#"
[[repository]]
name = "local"
mode = "spawn"
port = 3031
as = "alice"

[[repository]]
name = "remote"
mode = "attach"
port = 3032
where = "localhost:3000"
as = "bob"
"#,
        )
        .unwrap();

        let repos = config.repositories_to_start(None).unwrap();
        assert_eq!(repos.len(), 2);
        assert_eq!(repos[0].name, "local");
        assert_eq!(repos[1].name, "remote");
    }

    #[test]
    fn load_toml_reads_task_config() {
        let config = UserConfig::load_toml(
            r#"
[[repository]]
name = "local"
mode = "spawn"
port = 3031
as = "alice"

[[task]]
name = "build"

[[task.step]]
repository = "local"
plugin = "md-preview"
"#,
        )
        .unwrap();

        let task = config.find_task("build").unwrap();
        assert_eq!(task.step.len(), 1);
        assert_eq!(task.step[0].repository, "local");
        assert_eq!(task.step[0].plugin, "md-preview");
    }

    #[test]
    fn spawn_repository_requires_server_port() {
        let error = UserConfig::load_toml(
            r#"
[[repository]]
name = "local"
mode = "spawn"
port = 3031
as = "alice"

[repository.server]
plugin_url_prefix = "/.plugin2"
"#,
        )
        .unwrap()
        .validate()
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("repository.server.port is required when mode = \"spawn\"")
        );
    }
}
