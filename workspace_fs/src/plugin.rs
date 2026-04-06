use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;
use std::process::Stdio;

use anyhow::{Context, Result, bail};
use camino::Utf8PathBuf;
use serde::Deserialize;
use serde_json::Value as JsonValue;
use tokio::process::Command;

use crate::{
    config::{PluginConfig, RepositoryConfig, TaskConfig},
    identity::UserIdentity,
    path::WorkspacePath,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginTrigger {
    Get,
    Post,
    Put,
    Delete,
    Manual,
}

impl PluginTrigger {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Delete => "DELETE",
            Self::Manual => "manual",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PluginContext {
    pub repository_root: Utf8PathBuf,
    pub repository_name: String,
    pub plugin_name: String,
    pub output_directory: Utf8PathBuf,
    pub cache_directory: Utf8PathBuf,
    pub mount_url: Option<String>,
    pub dependency_mounts: BTreeMap<String, String>,
    pub path: Option<WorkspacePath>,
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

    pub async fn run_task(&self, task_name: &str, skip_deps: bool) -> Result<()> {
        let plan = self.plan_task(task_name, skip_deps)?;
        let plan_names = plan
            .iter()
            .map(|plugin| plugin.name.as_str())
            .collect::<Vec<_>>();
        tracing::info!(
            task = %task_name,
            skip_deps,
            plan = %plan_names.join(" -> "),
            "task execution plan"
        );

        for plugin in plan {
            self.run_plugin(plugin, PluginTrigger::Manual, None, &UserIdentity::new(""))
                .await?;
        }

        Ok(())
    }

    pub async fn run_manual_plugin(
        &self,
        plugin_name: &str,
        user_identity: &UserIdentity,
    ) -> Result<()> {
        let plugin = self
            .config
            .find_plugin(plugin_name)
            .with_context(|| format!("plugin not found: {plugin_name}"))?;

        if plugin.trigger != "manual" {
            bail!("plugin is not manual: {plugin_name}");
        }

        let mut visiting = BTreeSet::new();
        let mut executed = BTreeSet::new();
        self.run_plugin_with_dependencies(
            plugin,
            PluginTrigger::Manual,
            None,
            user_identity,
            &mut visiting,
            &mut executed,
        )
        .await
    }

    pub async fn run_hook_if_matched(
        &self,
        trigger: PluginTrigger,
        path: &WorkspacePath,
        user_identity: &UserIdentity,
    ) -> Result<()> {
        let mut executed = BTreeSet::new();
        for plugin in &self.config.plugin {
            if parse_trigger(&plugin.trigger)? != trigger {
                continue;
            }

            if !plugin_matches_path(plugin, path) {
                continue;
            }

            let mut visiting = BTreeSet::new();
            self.run_plugin_with_dependencies(
                plugin,
                trigger,
                Some(path),
                user_identity,
                &mut visiting,
                &mut executed,
            )
            .await?;
        }

        Ok(())
    }

    fn plan_task(&self, task_name: &str, skip_deps: bool) -> Result<Vec<&PluginConfig>> {
        let task = self
            .config
            .find_task(task_name)
            .with_context(|| format!("task not found: {task_name}"))?;
        let mut planned = BTreeSet::new();
        let mut plan = Vec::new();

        for step in &task.steps {
            let plugin = self
                .config
                .find_plugin(step)
                .with_context(|| format!("plugin not found for task step: {step}"))?;
            if skip_deps {
                if planned.insert(plugin.name.clone()) {
                    plan.push(plugin);
                }
                continue;
            }

            let mut visiting = BTreeSet::new();
            self.append_plugin_plan(plugin, &mut visiting, &mut planned, &mut plan)?;
        }

        Ok(plan)
    }

    fn append_plugin_plan<'b>(
        &'b self,
        plugin: &'b PluginConfig,
        visiting: &mut BTreeSet<String>,
        planned: &mut BTreeSet<String>,
        plan: &mut Vec<&'b PluginConfig>,
    ) -> Result<()> {
        if planned.contains(&plugin.name) {
            return Ok(());
        }
        if !visiting.insert(plugin.name.clone()) {
            bail!("plugin dependency cycle detected at {}", plugin.name);
        }

        for dependency_name in &plugin.deps {
            let dependency = self
                .config
                .find_plugin(dependency_name)
                .with_context(|| format!("plugin not found: {dependency_name}"))?;
            self.append_plugin_plan(dependency, visiting, planned, plan)?;
        }

        visiting.remove(&plugin.name);
        planned.insert(plugin.name.clone());
        plan.push(plugin);
        Ok(())
    }

    fn run_plugin_with_dependencies<'b>(
        &'b self,
        plugin: &'b PluginConfig,
        trigger: PluginTrigger,
        path: Option<&'b WorkspacePath>,
        user_identity: &'b UserIdentity,
        visiting: &'b mut BTreeSet<String>,
        executed: &'b mut BTreeSet<String>,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'b>> {
        Box::pin(async move {
            if executed.contains(&plugin.name) {
                return Ok(());
            }
            if !visiting.insert(plugin.name.clone()) {
                bail!("plugin dependency cycle detected at {}", plugin.name);
            }

            for dependency_name in &plugin.deps {
                let dependency = self
                    .config
                    .find_plugin(dependency_name)
                    .with_context(|| format!("plugin not found: {dependency_name}"))?;
                self.run_plugin_with_dependencies(
                    dependency,
                    PluginTrigger::Manual,
                    None,
                    user_identity,
                    visiting,
                    executed,
                )
                .await?;
            }

            self.run_plugin(plugin, trigger, path, user_identity)
                .await?;
            visiting.remove(&plugin.name);
            executed.insert(plugin.name.clone());
            Ok(())
        })
    }

    async fn run_plugin(
        &self,
        plugin: &PluginConfig,
        trigger: PluginTrigger,
        path: Option<&WorkspacePath>,
        user_identity: &UserIdentity,
    ) -> Result<()> {
        let path_str = path.map(WorkspacePath::as_str);
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
            dependency_mounts: dependency_mounts(self.config, plugin)?,
            path: path.cloned(),
            user_identity: user_identity.clone(),
        };

        tokio::fs::create_dir_all(context.output_directory.as_std_path())
            .await
            .context("failed to create plugin output directory")?;
        tokio::fs::create_dir_all(context.cache_directory.as_std_path())
            .await
            .context("failed to create plugin cache directory")?;

        let plugin_command = resolve_plugin_command(plugin)?;
        let program = expand_placeholder(&plugin_command[0], &context, trigger)?;
        let args = plugin_command[1..]
            .iter()
            .map(|arg| expand_placeholder(arg, &context, trigger))
            .collect::<Result<Vec<_>>>()?;
        let settings_json = resolved_plugin_settings_json(plugin, &context, trigger)?;

        tracing::info!(
            plugin = %plugin.name,
            trigger = %trigger.as_str(),
            path = %path_str.unwrap_or(""),
            "running plugin"
        );

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
            .env("WORKSPACE_FS_TRIGGER", trigger.as_str())
            .env("WORKSPACE_FS_USER_IDENTITY", context.user_identity.as_str())
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        if let Some(mount_url) = &context.mount_url {
            command.env("MOUNT_URL", mount_url);
        }
        for (name, mount_url) in &context.dependency_mounts {
            command.env(name, mount_url);
        }
        if let Some(path) = path_str {
            command.env("WORKSPACE_FS_PATH", path);
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
    #[serde(default)]
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
        _ => bail!("unsupported plugin runner: {}", plugin.runner),
    }
}

fn resolve_default_plugin_command(plugin_name: &str) -> Result<Vec<String>> {
    let default_config_path = Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("default.toml");
    let text = std::fs::read_to_string(default_config_path.as_std_path())
        .context("failed to read workspace_fs/default.toml")?;
    let config: DefaultPluginConfig =
        toml::from_str(&text).context("failed to parse workspace_fs/default.toml")?;

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

fn plugin_matches_path(plugin: &PluginConfig, path: &WorkspacePath) -> bool {
    let Some(plugin_path) = &plugin.path else {
        return false;
    };

    if plugin_path.as_str() == "." {
        return true;
    }

    if plugin_path.is_directory() {
        return path.starts_with(plugin_path);
    }

    plugin_path.as_str() == path.as_str()
}

fn parse_trigger(trigger: &str) -> Result<PluginTrigger> {
    match trigger {
        "GET" => Ok(PluginTrigger::Get),
        "POST" => Ok(PluginTrigger::Post),
        "PUT" => Ok(PluginTrigger::Put),
        "DELETE" => Ok(PluginTrigger::Delete),
        "manual" => Ok(PluginTrigger::Manual),
        _ => bail!("unsupported plugin trigger: {trigger}"),
    }
}

fn expand_placeholder(
    input: &str,
    context: &PluginContext,
    trigger: PluginTrigger,
) -> Result<String> {
    let mut value = input.to_owned();
    if context.path.is_none() && contains_path_placeholder(input) {
        bail!("path placeholder requires request path: {input}");
    }

    let replacements = [
        ("{REPOSITORY_ROOT}", context.repository_root.as_str()),
        ("{REPOSITORY_NAME}", context.repository_name.as_str()),
        ("{PLUGIN_NAME}", context.plugin_name.as_str()),
        ("{OUTPOST_DIRECTORY}", context.output_directory.as_str()),
        ("{OUTPUT_DIRECTORY}", context.output_directory.as_str()),
        ("{WORKSPACE_FS_ROOT}", env!("CARGO_MANIFEST_DIR")),
    ];

    for (from, to) in replacements {
        value = value.replace(from, to);
    }
    let default_plugins_root =
        Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../default_plugins");
    value = value.replace("{DEFAULT_PLUGINS_ROOT}", default_plugins_root.as_str());
    if let Some(mount_url) = &context.mount_url {
        value = value.replace("{MOUNT_URL}", mount_url);
    }
    for (name, mount_url) in &context.dependency_mounts {
        value = value.replace(&format!("{{{name}}}"), mount_url);
    }

    if let Some((path_placeholder, user_placeholder)) = request_placeholders(trigger) {
        let path = context
            .path
            .as_ref()
            .map(WorkspacePath::as_str)
            .unwrap_or("");
        value = value.replace(path_placeholder, path);
        value = value.replace(user_placeholder, context.user_identity.as_str());
    }

    if value.contains('{') || value.contains('}') {
        bail!("unknown placeholder in plugin command: {input}");
    }

    Ok(value)
}

fn resolved_plugin_settings_json(
    plugin: &PluginConfig,
    context: &PluginContext,
    trigger: PluginTrigger,
) -> Result<String> {
    let resolved = plugin
        .extra
        .iter()
        .map(|(key, value)| {
            resolve_plugin_setting_value(value, context, trigger)
                .map(|resolved| (key.clone(), resolved))
        })
        .collect::<Result<serde_json::Map<String, JsonValue>>>()?;
    serde_json::to_string(&JsonValue::Object(resolved))
        .context("failed to serialize plugin settings json")
}

fn resolve_plugin_setting_value(
    value: &toml::Value,
    context: &PluginContext,
    trigger: PluginTrigger,
) -> Result<JsonValue> {
    match value {
        toml::Value::String(text) => Ok(JsonValue::String(expand_placeholder(
            text, context, trigger,
        )?)),
        toml::Value::Integer(number) => Ok(JsonValue::from(*number)),
        toml::Value::Float(number) => serde_json::Number::from_f64(*number)
            .map(JsonValue::Number)
            .ok_or_else(|| anyhow::anyhow!("plugin setting contains non-finite float")),
        toml::Value::Boolean(value) => Ok(JsonValue::Bool(*value)),
        toml::Value::Datetime(value) => Ok(JsonValue::String(value.to_string())),
        toml::Value::Array(values) => values
            .iter()
            .map(|item| resolve_plugin_setting_value(item, context, trigger))
            .collect::<Result<Vec<_>>>()
            .map(JsonValue::Array),
        toml::Value::Table(table) => table
            .iter()
            .map(|(key, value)| {
                resolve_plugin_setting_value(value, context, trigger)
                    .map(|resolved| (key.clone(), resolved))
            })
            .collect::<Result<serde_json::Map<String, JsonValue>>>()
            .map(JsonValue::Object),
    }
}

fn dependency_mounts(
    config: &RepositoryConfig,
    plugin: &PluginConfig,
) -> Result<BTreeMap<String, String>> {
    let mut mounts = BTreeMap::new();
    for dependency_name in &plugin.deps {
        let dependency = config
            .find_plugin(dependency_name)
            .with_context(|| format!("plugin not found: {dependency_name}"))?;
        let Some(mount) = &dependency.mount else {
            continue;
        };
        mounts.insert(mount_env_name(dependency_name), mount.clone());
    }
    Ok(mounts)
}

fn mount_env_name(plugin_name: &str) -> String {
    let mut value = String::from("MOUNT_");
    for ch in plugin_name.chars() {
        if ch.is_ascii_alphanumeric() {
            value.push(ch.to_ascii_uppercase());
        } else {
            value.push('_');
        }
    }
    value
}

fn contains_path_placeholder(input: &str) -> bool {
    ["{GET.PATH}", "{POST.PATH}", "{PUT.PATH}", "{DELETE.PATH}"]
        .into_iter()
        .any(|placeholder| input.contains(placeholder))
}

fn request_placeholders(trigger: PluginTrigger) -> Option<(&'static str, &'static str)> {
    match trigger {
        PluginTrigger::Get => Some(("{GET.PATH}", "{GET.USER-IDENTITY}")),
        PluginTrigger::Post => Some(("{POST.PATH}", "{POST.USER-IDENTITY}")),
        PluginTrigger::Put => Some(("{PUT.PATH}", "{PUT.USER-IDENTITY}")),
        PluginTrigger::Delete => Some(("{DELETE.PATH}", "{DELETE.USER-IDENTITY}")),
        PluginTrigger::Manual => None,
    }
}

#[allow(dead_code)]
fn _task_reference(_task: &TaskConfig) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin_context(path: Option<&str>) -> PluginContext {
        PluginContext {
            repository_root: Utf8PathBuf::from("/repo"),
            repository_name: "repo".into(),
            plugin_name: "plugin".into(),
            output_directory: Utf8PathBuf::from("/repo/.repo/plugin/generated"),
            cache_directory: Utf8PathBuf::from("/repo/.repo/plugin/cache"),
            mount_url: Some("/plugin-assets/".into()),
            dependency_mounts: BTreeMap::from([(
                "MOUNT_BUILD_WASM".into(),
                "/wasm_bundle/".into(),
            )]),
            path: path.map(|value| WorkspacePath::from_url(&format!("/{value}")).unwrap()),
            user_identity: UserIdentity::new("user"),
        }
    }

    fn plugin_config_with_extra(extra: BTreeMap<String, toml::Value>) -> PluginConfig {
        PluginConfig {
            name: "plugin".into(),
            runner: "command".into(),
            command: vec!["echo".into()],
            trigger: "manual".into(),
            path: None,
            deps: vec!["build-wasm".into()],
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
    fn expand_placeholder_rejects_path_placeholder_without_path() {
        let error = expand_placeholder("{GET.PATH}", &plugin_context(None), PluginTrigger::Manual)
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("path placeholder requires request path")
        );
    }

    #[test]
    fn expand_placeholder_replaces_trigger_specific_values() {
        let value = expand_placeholder(
            "{REPOSITORY_ROOT}:{POST.PATH}:{POST.USER-IDENTITY}:{MOUNT_URL}:{MOUNT_BUILD_WASM}",
            &plugin_context(Some("docs/a.md")),
            PluginTrigger::Post,
        )
        .unwrap();

        assert_eq!(value, "/repo:docs/a.md:user:/plugin-assets/:/wasm_bundle/");
    }

    #[test]
    fn expand_placeholder_rejects_mismatched_trigger_placeholder() {
        let error = expand_placeholder(
            "{GET.USER-IDENTITY}",
            &plugin_context(Some("docs/a.md")),
            PluginTrigger::Manual,
        )
        .unwrap_err();

        assert!(error.to_string().contains("unknown placeholder"));
    }

    #[test]
    fn mount_env_name_normalizes_plugin_names() {
        assert_eq!(mount_env_name("build-wasm"), "MOUNT_BUILD_WASM");
        assert_eq!(mount_env_name("Build_2"), "MOUNT_BUILD_2");
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
                        toml::Value::String("{MOUNT_BUILD_WASM}enhance.js".into()),
                    ),
                    (
                        "settings".into(),
                        toml::Value::Table(toml::map::Map::from_iter([(
                            "mount".into(),
                            toml::Value::String("{MOUNT_BUILD_WASM}".into()),
                        )])),
                    ),
                ]))]),
            )])),
        )]));

        let json =
            resolved_plugin_settings_json(&plugin, &plugin_context(None), PluginTrigger::Manual)
                .unwrap();

        assert_eq!(
            json,
            r#"{"md_preview":{"enhance":[{"name":"embedded-models","settings":{"mount":"/wasm_bundle/"},"url":"/wasm_bundle/enhance.js"}]}}"#
        );
    }

    #[test]
    fn plugin_matches_path_uses_exact_match_for_files() {
        let plugin = PluginConfig {
            name: "plugin".into(),
            runner: "command".into(),
            command: vec!["echo".into()],
            trigger: "GET".into(),
            path: Some(WorkspacePath::from_path_str("docs/a.md").unwrap()),
            deps: Vec::new(),
            mount: None,
            extra: Default::default(),
        };

        assert!(plugin_matches_path(
            &plugin,
            &WorkspacePath::from_path_str("docs/a.md").unwrap()
        ));
        assert!(!plugin_matches_path(
            &plugin,
            &WorkspacePath::from_path_str("docs/b.md").unwrap()
        ));
    }

    #[test]
    fn plugin_matches_path_uses_prefix_match_for_directories() {
        let plugin = PluginConfig {
            name: "plugin".into(),
            runner: "command".into(),
            command: vec!["echo".into()],
            trigger: "GET".into(),
            path: Some(WorkspacePath::from_path_str("docs/").unwrap()),
            deps: Vec::new(),
            mount: None,
            extra: Default::default(),
        };

        assert!(plugin_matches_path(
            &plugin,
            &WorkspacePath::from_path_str("docs/a.md").unwrap()
        ));
        assert!(!plugin_matches_path(
            &plugin,
            &WorkspacePath::from_path_str("images/a.md").unwrap()
        ));
    }

    #[test]
    fn plan_task_orders_dependencies_before_steps() {
        let config = RepositoryConfig {
            name: "repo".into(),
            serve: crate::config::ServeSettings::default(),
            policy: Vec::new(),
            plugin: vec![
                PluginConfig {
                    name: "build-wasm".into(),
                    runner: "command".into(),
                    command: vec!["echo".into()],
                    trigger: "manual".into(),
                    path: None,
                    deps: Vec::new(),
                    mount: None,
                    extra: Default::default(),
                },
                PluginConfig {
                    name: "md-preview".into(),
                    runner: "command".into(),
                    command: vec!["echo".into()],
                    trigger: "manual".into(),
                    path: None,
                    deps: vec!["build-wasm".into()],
                    mount: None,
                    extra: Default::default(),
                },
            ],
            task: vec![TaskConfig {
                name: "build".into(),
                steps: vec!["md-preview".into()],
            }],
        };
        let runner = PluginRunner::new(camino::Utf8Path::new("/repo"), "repo", &config);

        let plan = runner.plan_task("build", false).unwrap();
        let names = plan
            .iter()
            .map(|plugin| plugin.name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(names, vec!["build-wasm", "md-preview"]);
    }

    #[test]
    fn plan_task_skip_deps_uses_task_steps_only() {
        let config = RepositoryConfig {
            name: "repo".into(),
            serve: crate::config::ServeSettings::default(),
            policy: Vec::new(),
            plugin: vec![
                PluginConfig {
                    name: "build-wasm".into(),
                    runner: "command".into(),
                    command: vec!["echo".into()],
                    trigger: "manual".into(),
                    path: None,
                    deps: Vec::new(),
                    mount: None,
                    extra: Default::default(),
                },
                PluginConfig {
                    name: "md-preview".into(),
                    runner: "command".into(),
                    command: vec!["echo".into()],
                    trigger: "manual".into(),
                    path: None,
                    deps: vec!["build-wasm".into()],
                    mount: None,
                    extra: Default::default(),
                },
            ],
            task: vec![TaskConfig {
                name: "build".into(),
                steps: vec!["md-preview".into()],
            }],
        };
        let runner = PluginRunner::new(camino::Utf8Path::new("/repo"), "repo", &config);

        let plan = runner.plan_task("build", true).unwrap();
        let names = plan
            .iter()
            .map(|plugin| plugin.name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(names, vec!["md-preview"]);
    }
}
