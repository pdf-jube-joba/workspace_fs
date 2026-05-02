use anyhow::{Result, anyhow, bail};

use crate::{
    config::user_config::UserConfig, runtime::app::ServerSupervisor, task_runner::task_runner,
};

pub(crate) async fn run_repl(config: &UserConfig, supervisor: &mut ServerSupervisor) -> Result<()> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let stdin = tokio::io::stdin();
    let mut lines = BufReader::new(stdin).lines();
    let mut stdout = tokio::io::stdout();

    loop {
        stdout.write_all(b"> ").await?;
        stdout.flush().await?;

        let Some(line) = lines.next_line().await? else {
            break;
        };
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if matches!(line, "exit" | "quit") {
            break;
        }

        match parse_repl_command(line) {
            Ok(ReplCommand::Task(task_name)) => {
                if let Err(error) = task_runner::run_task(config, &task_name, supervisor).await {
                    eprintln!("{error}");
                }
            }
            Ok(ReplCommand::Plugin {
                repository_name,
                plugin_name,
            }) => {
                if let Err(error) = task_runner::run_repository_plugin(
                    config,
                    supervisor,
                    &repository_name,
                    &plugin_name,
                )
                .await
                {
                    eprintln!("{error}");
                }
            }
            Ok(ReplCommand::Help) => {
                stdout
                    .write_all(b"task <task-name>\nplugin <repository-name> <plugin-name>\nexit\n")
                    .await?;
                stdout.flush().await?;
            }
            Err(error) => {
                eprintln!("{error}");
            }
        }
    }

    Ok(())
}

enum ReplCommand {
    Task(String),
    Plugin {
        repository_name: String,
        plugin_name: String,
    },
    Help,
}

fn parse_repl_command(line: &str) -> Result<ReplCommand> {
    let mut parts = line.split_whitespace();
    let Some(command) = parts.next() else {
        return Ok(ReplCommand::Help);
    };

    match command {
        "task" => {
            let task_name = parts
                .next()
                .ok_or_else(|| anyhow!("usage: task <task-name>"))?;
            if parts.next().is_some() {
                bail!("usage: task <task-name>");
            }
            Ok(ReplCommand::Task(task_name.to_owned()))
        }
        "plugin" => {
            let repository_name = parts
                .next()
                .ok_or_else(|| anyhow!("usage: plugin <repository-name> <plugin-name>"))?;
            let plugin_name = parts
                .next()
                .ok_or_else(|| anyhow!("usage: plugin <repository-name> <plugin-name>"))?;
            if parts.next().is_some() {
                bail!("usage: plugin <repository-name> <plugin-name>");
            }
            Ok(ReplCommand::Plugin {
                repository_name: repository_name.to_owned(),
                plugin_name: plugin_name.to_owned(),
            })
        }
        "help" => Ok(ReplCommand::Help),
        _ => bail!("unknown command: {command}"),
    }
}
