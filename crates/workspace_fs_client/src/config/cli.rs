use anyhow::{Result, anyhow, bail};
use camino::Utf8PathBuf;

#[derive(Debug, Clone)]
pub struct CliOptions {
    pub repository_path: Option<Utf8PathBuf>,
    pub repository_name: Option<String>,
    pub task: Option<String>,
    pub task_only: Option<String>,
    pub repl: bool,
}

pub fn parse_cli_options<I>(args: I) -> Result<CliOptions>
where
    I: IntoIterator<Item = String>,
{
    let mut args = args.into_iter();
    let mut repository_path = None;
    let mut repository_name = None;
    let mut task = None;
    let mut task_only = None;
    let mut repl = false;

    while let Some(arg) = args.next() {
        if repository_path.is_none() && !arg.starts_with("--") {
            repository_path = Some(Utf8PathBuf::from(arg));
            continue;
        }
        match arg.as_str() {
            "--repository" => {
                repository_name = Some(
                    args.next()
                        .ok_or_else(|| anyhow!("missing value for --repository"))?,
                );
            }
            "--task" => {
                task = Some(
                    args.next()
                        .ok_or_else(|| anyhow!("missing value for --task"))?,
                );
            }
            "--task-only" => {
                task_only = Some(
                    args.next()
                        .ok_or_else(|| anyhow!("missing value for --task-only"))?,
                );
            }
            "--repl" => {
                repl = true;
            }
            _ => bail!("unknown argument: {arg}"),
        }
    }

    if task.is_some() && task_only.is_some() {
        bail!("--task and --task-only are mutually exclusive");
    }
    if repl && (task.is_some() || task_only.is_some()) {
        bail!("--repl cannot be combined with --task or --task-only");
    }

    Ok(CliOptions {
        repository_path,
        repository_name,
        task,
        task_only,
        repl,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cli_options_accepts_missing_repository_path() {
        let cli = parse_cli_options(["--task".to_string(), "build".to_string()]).unwrap();

        assert!(cli.repository_path.is_none());
        assert_eq!(cli.task.as_deref(), Some("build"));
    }

    #[test]
    fn parse_cli_options_accepts_repository_path() {
        let cli = parse_cli_options([
            "./repo".to_string(),
            "--repository".to_string(),
            "local".to_string(),
        ])
        .unwrap();

        assert_eq!(
            cli.repository_path.as_ref().map(|path| path.as_str()),
            Some("./repo")
        );
        assert_eq!(cli.repository_name.as_deref(), Some("local"));
    }

    #[test]
    fn parse_cli_options_accepts_repl() {
        let cli = parse_cli_options(["--repl".to_string()]).unwrap();

        assert!(cli.repl);
        assert!(cli.task.is_none());
        assert!(cli.task_only.is_none());
    }
}
