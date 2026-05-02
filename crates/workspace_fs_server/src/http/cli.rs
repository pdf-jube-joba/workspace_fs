use anyhow::{Context, Result, anyhow, bail};
use camino::Utf8PathBuf;

use crate::infra::repository_config::ServeSettingsOverride;

#[derive(Debug, Clone)]
pub(crate) struct CliOptions {
    pub repository_path: Utf8PathBuf,
    pub serve_overrides: ServeSettingsOverride,
}

pub(crate) fn parse_cli_options<I>(args: I) -> Result<CliOptions>
where
    I: IntoIterator<Item = String>,
{
    let mut args = args.into_iter();
    let repository_path = args
        .next()
        .ok_or_else(|| anyhow!("usage: workspace_fs <repository-path>"))?;
    let mut serve_overrides = ServeSettingsOverride::default();

    for arg in args {
        let Some((name, value)) = arg.split_once('=') else {
            bail!("unknown argument: {arg}");
        };
        match name {
            "--port" => {
                serve_overrides.port = Some(
                    value
                        .parse()
                        .with_context(|| format!("invalid value for --port: {value}"))?,
                );
            }
            "--plugin-url-prefix" => {
                serve_overrides.plugin_url_prefix = Some(value.to_owned());
            }
            "--policy-url-prefix" => {
                serve_overrides.policy_url_prefix = Some(value.to_owned());
            }
            "--info-url-prefix" => {
                serve_overrides.info_url_prefix = Some(value.to_owned());
            }
            _ => bail!("unknown argument: {arg}"),
        }
    }

    Ok(CliOptions {
        repository_path: Utf8PathBuf::from(repository_path),
        serve_overrides,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cli_args_accepts_repository_only() {
        let cli = parse_cli_options(["./repo".to_string()]).unwrap();

        assert_eq!(cli.repository_path, Utf8PathBuf::from("./repo"));
        assert!(cli.serve_overrides.port.is_none());
    }

    #[test]
    fn parse_cli_args_accepts_serve_overrides() {
        let cli = parse_cli_options([
            "./repo".to_string(),
            "--port=4010".to_string(),
            "--plugin-url-prefix=/.plugin2".to_string(),
            "--policy-url-prefix=/.policy2".to_string(),
            "--info-url-prefix=/.info2".to_string(),
        ])
        .unwrap();

        assert_eq!(cli.repository_path, Utf8PathBuf::from("./repo"));
        assert_eq!(cli.serve_overrides.port, Some(4010));
        assert_eq!(
            cli.serve_overrides.plugin_url_prefix.as_deref(),
            Some("/.plugin2")
        );
        assert_eq!(
            cli.serve_overrides.policy_url_prefix.as_deref(),
            Some("/.policy2")
        );
        assert_eq!(
            cli.serve_overrides.info_url_prefix.as_deref(),
            Some("/.info2")
        );
    }
}
