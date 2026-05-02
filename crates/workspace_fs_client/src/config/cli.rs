use anyhow::{Result, bail};
use camino::Utf8PathBuf;

#[derive(Debug, Clone)]
pub struct CliOptions {
    pub repository_path: Option<Utf8PathBuf>,
}

pub fn parse_cli_options<I>(args: I) -> Result<CliOptions>
where
    I: IntoIterator<Item = String>,
{
    let mut args = args.into_iter();
    let mut repository_path = None;

    while let Some(arg) = args.next() {
        if repository_path.is_none() && !arg.starts_with("--") {
            repository_path = Some(Utf8PathBuf::from(arg));
            continue;
        }
        bail!("unknown argument: {arg}");
    }
    Ok(CliOptions {
        repository_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cli_options_accepts_missing_repository_path() {
        let cli = parse_cli_options(Vec::<String>::new()).unwrap();

        assert!(cli.repository_path.is_none());
    }

    #[test]
    fn parse_cli_options_accepts_repository_path() {
        let cli = parse_cli_options(["./repo".to_string()]).unwrap();

        assert_eq!(
            cli.repository_path.as_ref().map(|path| path.as_str()),
            Some("./repo")
        );
    }

    #[test]
    fn parse_cli_options_rejects_named_flags() {
        let error = parse_cli_options(["--task".to_string(), "build".to_string()]).unwrap_err();
        assert!(error.to_string().contains("unknown argument: --task"));
    }
}
