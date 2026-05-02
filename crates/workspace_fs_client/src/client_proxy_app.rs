use std::{collections::HashMap, env, process::Stdio, sync::Arc};

use anyhow::{Context, Result, anyhow, bail};
use axum::{
    Router,
    body::{Body, Bytes},
    extract::{OriginalUri, State},
    http::{HeaderMap, HeaderName, Method, Response, StatusCode, Uri, header},
    routing::any,
};
use camino::Utf8PathBuf;
use reqwest::Client;
use tokio::{
    net::TcpStream,
    process::{Child, Command},
    signal,
    sync::watch,
    task::JoinSet,
    time::{Duration, Instant, sleep},
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::user_config::{UserConfig, UserRepositoryConfig, UserServeConfig};

#[derive(Debug, Clone)]
pub struct CliOptions {
    pub repository_path: Option<Utf8PathBuf>,
    pub repository_name: Option<String>,
    pub task: Option<String>,
    pub task_only: Option<String>,
    pub repl: bool,
}

#[derive(Clone)]
struct AppState {
    client: Client,
    repository_name: String,
    upstream_base: String,
    user_identity: String,
}

struct SpawnedServer {
    child: Child,
    upstream_base: String,
}

struct ServerSupervisor {
    workspace_root: Utf8PathBuf,
    repository_root: Utf8PathBuf,
    spawned: HashMap<String, SpawnedServer>,
}

pub async fn run_from_env() -> Result<()> {
    init_tracing();
    let cli = parse_cli_options(env::args().skip(1))?;
    run(cli).await
}

pub async fn run(cli: CliOptions) -> Result<()> {
    let repository_root = resolve_repository_root(cli.repository_path.as_ref())?;
    let config = UserConfig::load(&repository_root)?;
    let workspace_root = workspace_fs_root();
    let mut supervisor = ServerSupervisor::new(workspace_root, repository_root.clone());

    let task_name = cli.task.clone().or(cli.task_only.clone());
    if let Some(task_name) = task_name.as_deref() {
        run_task(&config, task_name, &mut supervisor).await?;
        if cli.task_only.is_some() {
            supervisor.shutdown_all().await;
            return Ok(());
        }
    }

    if cli.repl {
        run_repl(&config, &mut supervisor).await?;
        supervisor.shutdown_all().await;
        return Ok(());
    }

    let repositories = config.repositories_to_start(cli.repository_name.as_deref())?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let shutdown_for_signal = shutdown_tx.clone();
    tokio::spawn(async move {
        let _ = signal::ctrl_c().await;
        let _ = shutdown_for_signal.send(true);
    });

    let mut join_set = JoinSet::new();
    for repository in repositories {
        let upstream_base = supervisor.upstream_base_for_repository(repository).await?;
        let state = Arc::new(AppState {
            client: Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .context("failed to build proxy client")?,
            repository_name: repository.name.clone(),
            upstream_base,
            user_identity: repository.as_user.clone(),
        });

        tracing::info!(
            repository = %repository_root,
            target_repository = %state.repository_name,
            listen_port = repository.port,
            upstream = %state.upstream_base,
            user_identity = %state.user_identity,
            "client proxy configuration loaded"
        );

        join_set.spawn(run_proxy_server(
            repository.port,
            state,
            shutdown_rx.clone(),
        ));
    }

    let mut first_error: Option<anyhow::Error> = None;
    while let Some(result) = join_set.join_next().await {
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                if first_error.is_none() {
                    first_error = Some(error);
                }
                let _ = shutdown_tx.send(true);
            }
            Err(error) => {
                if first_error.is_none() {
                    first_error = Some(anyhow!("proxy server task failed: {error}"));
                }
                let _ = shutdown_tx.send(true);
            }
        }
    }

    supervisor.shutdown_all().await;
    if let Some(error) = first_error {
        return Err(error);
    }
    Ok(())
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

fn init_tracing() {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            env::var("RUST_LOG").unwrap_or_else(|_| "workspace_fs=info,tower_http=info".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();
}

async fn proxy_handler(
    State(state): State<Arc<AppState>>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response<Body> {
    match forward_request(state, method, uri, headers, body).await {
        Ok(response) => response,
        Err(error) => {
            tracing::warn!(error = %error, "proxy request failed");
            Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(Body::from("proxy request failed"))
                .unwrap()
        }
    }
}

fn resolve_repository_root(path: Option<&Utf8PathBuf>) -> Result<Utf8PathBuf> {
    let base = match path {
        Some(path) => path.clone(),
        None => Utf8PathBuf::from_path_buf(
            std::env::current_dir().context("failed to get current directory")?,
        )
        .map_err(|_| anyhow!("current directory must be UTF-8"))?,
    };
    Ok(base.canonicalize_utf8()?)
}

async fn forward_request(
    state: Arc<AppState>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response<Body>> {
    let path_and_query = uri
        .path_and_query()
        .map(|value| value.as_str())
        .unwrap_or("/");
    let target_url = format!("{}{}", state.upstream_base, path_and_query);
    let reqwest_method = reqwest::Method::from_bytes(method.as_str().as_bytes())
        .context("unsupported http method")?;

    let mut request = state.client.request(reqwest_method, target_url);
    for (name, value) in &headers {
        if is_skipped_request_header(name) {
            continue;
        }
        request = request.header(name, value);
    }
    request = request.header("user-identity", &state.user_identity);

    let upstream = request
        .body(body)
        .send()
        .await
        .context("upstream request failed")?;
    let status = upstream.status();
    let upstream_headers = upstream.headers().clone();
    let body = upstream
        .bytes()
        .await
        .context("failed to read upstream body")?;

    let mut response = Response::builder().status(status);
    for (name, value) in &upstream_headers {
        if is_skipped_response_header(name) {
            continue;
        }
        response = response.header(name, value);
    }

    response
        .body(Body::from(body))
        .map_err(|error| anyhow!("failed to build proxy response: {error}"))
}

async fn run_task(
    config: &UserConfig,
    task_name: &str,
    supervisor: &mut ServerSupervisor,
) -> Result<()> {
    let task = config
        .find_task(task_name)
        .ok_or_else(|| anyhow!("task not found in .repo/user.toml: {task_name}"))?;
    let client = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .context("failed to build task client")?;

    for step in &task.step {
        let repository = config.find_repository(&step.repository).ok_or_else(|| {
            anyhow!(
                "repository not found in .repo/user.toml: {}",
                step.repository
            )
        })?;
        let upstream_base = supervisor.upstream_base_for_repository(repository).await?;
        run_task_step(&client, repository, &upstream_base, &step.plugin).await?;
    }

    Ok(())
}

async fn run_task_step(
    client: &Client,
    repository: &UserRepositoryConfig,
    upstream_base: &str,
    plugin_name: &str,
) -> Result<()> {
    let plugin_url_prefix = repository.upstream_plugin_url_prefix();
    let plugin_url = format!("{}{}/{}/run", upstream_base, plugin_url_prefix, plugin_name);
    let response = client
        .post(plugin_url)
        .header("user-identity", &repository.as_user)
        .send()
        .await
        .with_context(|| format!("failed to invoke plugin over HTTP: {}", plugin_name))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        bail!(
            "plugin invocation failed: repository={} plugin={} status={} body={}",
            repository.name,
            plugin_name,
            status,
            body
        );
    }

    Ok(())
}

async fn run_repl(config: &UserConfig, supervisor: &mut ServerSupervisor) -> Result<()> {
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
                if let Err(error) = run_task(config, &task_name, supervisor).await {
                    eprintln!("{error}");
                }
            }
            Ok(ReplCommand::Plugin {
                repository_name,
                plugin_name,
            }) => {
                if let Err(error) =
                    run_repository_plugin(config, supervisor, &repository_name, &plugin_name).await
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

async fn run_repository_plugin(
    config: &UserConfig,
    supervisor: &mut ServerSupervisor,
    repository_name: &str,
    plugin_name: &str,
) -> Result<()> {
    let repository = config
        .find_repository(repository_name)
        .ok_or_else(|| anyhow!("repository not found in .repo/user.toml: {repository_name}"))?;
    let upstream_base = supervisor.upstream_base_for_repository(repository).await?;
    let client = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .context("failed to build plugin client")?;
    run_task_step(&client, repository, &upstream_base, plugin_name).await
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

async fn run_proxy_server(
    port: u16,
    state: Arc<AppState>,
    shutdown: watch::Receiver<bool>,
) -> Result<()> {
    let app = Router::new()
        .route("/", any(proxy_handler))
        .route("/{*path}", any(proxy_handler))
        .with_state(state);

    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    tracing::info!("client proxy listening on http://{addr}");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(wait_for_shutdown(shutdown))
        .await
        .context("client proxy server failed")
}

async fn wait_for_shutdown(mut shutdown: watch::Receiver<bool>) {
    loop {
        if *shutdown.borrow() {
            break;
        }
        if shutdown.changed().await.is_err() {
            break;
        }
    }
}

impl ServerSupervisor {
    fn new(workspace_root: Utf8PathBuf, repository_root: Utf8PathBuf) -> Self {
        Self {
            workspace_root,
            repository_root,
            spawned: HashMap::new(),
        }
    }

    async fn upstream_base_for_repository(
        &mut self,
        repository: &UserRepositoryConfig,
    ) -> Result<String> {
        if let Some(where_) = &repository.where_ {
            return normalize_upstream_base(where_);
        }

        self.ensure_spawned(repository).await
    }

    async fn ensure_spawned(&mut self, repository: &UserRepositoryConfig) -> Result<String> {
        if let Some(existing) = self.spawned.get(&repository.name) {
            return Ok(existing.upstream_base.clone());
        }

        let server = repository.effective_serve_config();
        let upstream_base = format!("http://127.0.0.1:{}", server.normalize_port());
        let child = self.spawn_server_process(&server).await?;
        self.spawned.insert(
            repository.name.clone(),
            SpawnedServer {
                child,
                upstream_base: upstream_base.clone(),
            },
        );
        Ok(upstream_base)
    }

    async fn spawn_server_process(&self, server: &UserServeConfig) -> Result<Child> {
        self.build_server_binary().await?;
        let binary = self.server_binary_path();
        let mut command = Command::new(binary.as_std_path());
        command
            .arg(self.repository_root.as_str())
            .arg(format!("--port={}", server.normalize_port()))
            .arg(format!("--plugin-url-prefix={}", server.plugin_url_prefix))
            .arg(format!("--policy-url-prefix={}", server.policy_url_prefix))
            .arg(format!("--info-url-prefix={}", server.info_url_prefix))
            .args(&server.args)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());

        let mut child = command.spawn().context("failed to launch server binary")?;
        self.wait_for_server_ready(&mut child, server.normalize_port())
            .await?;
        Ok(child)
    }

    async fn build_server_binary(&self) -> Result<()> {
        let status = Command::new("cargo")
            .arg("build")
            .arg("-p")
            .arg("workspace_fs_server")
            .current_dir(self.workspace_root.as_std_path())
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .await
            .context("failed to build workspace_fs_server")?;

        if !status.success() {
            return Err(anyhow!(
                "cargo build for workspace_fs_server failed: {status}"
            ));
        }

        Ok(())
    }

    fn server_binary_path(&self) -> Utf8PathBuf {
        let name = if env::consts::EXE_EXTENSION.is_empty() {
            "workspace_fs_server".to_owned()
        } else {
            format!("workspace_fs_server.{}", env::consts::EXE_EXTENSION)
        };
        self.workspace_root.join("target").join("debug").join(name)
    }

    async fn wait_for_server_ready(&self, child: &mut Child, port: u16) -> Result<()> {
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
        let deadline = Instant::now() + Duration::from_secs(15);

        loop {
            if let Some(status) = child.try_wait().context("failed to check server status")? {
                return Err(anyhow!("server process exited with {status}"));
            }

            match TcpStream::connect(addr).await {
                Ok(stream) => {
                    drop(stream);
                    return Ok(());
                }
                Err(error) if is_connection_pending(&error) => {
                    if Instant::now() >= deadline {
                        return Err(anyhow!("timed out waiting for server to start"));
                    }
                    sleep(Duration::from_millis(100)).await;
                }
                Err(error) => return Err(error).context("failed to connect to spawned server"),
            }
        }
    }

    async fn shutdown_all(&mut self) {
        for spawned in self.spawned.values_mut() {
            terminate_child(&mut spawned.child).await;
        }
        self.spawned.clear();
    }
}

async fn terminate_child(child: &mut Child) {
    if let Ok(Some(_)) = child.try_wait() {
        return;
    }

    if let Err(error) = child.kill().await {
        tracing::warn!(error = %error, pid = child.id().unwrap_or_default(), "failed to kill child process");
        return;
    }

    if let Err(error) = child.wait().await {
        tracing::warn!(error = %error, pid = child.id().unwrap_or_default(), "failed to wait for child process");
    }
}

fn is_connection_pending(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::ConnectionRefused
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::TimedOut
            | std::io::ErrorKind::NotConnected
    )
}

fn is_skipped_request_header(name: &HeaderName) -> bool {
    name == header::HOST || is_hop_by_hop_header(name) || name == "user-identity"
}

fn is_skipped_response_header(name: &HeaderName) -> bool {
    is_hop_by_hop_header(name)
}

fn is_hop_by_hop_header(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

fn normalize_upstream_base(input: &str) -> Result<String> {
    let trimmed = input.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        bail!("repository.where must not be empty");
    }
    if trimmed.starts_with("https://") {
        bail!("https is not supported for repository.where");
    }
    if let Some((scheme, _)) = trimmed.split_once("://") {
        if scheme != "http" {
            bail!("unsupported scheme for repository.where: {scheme}");
        }
        return Ok(trimmed.to_owned());
    }
    Ok(format!("http://{trimmed}"))
}

fn workspace_fs_root() -> Utf8PathBuf {
    let manifest_dir = Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if let Some(root) = manifest_dir.parent().and_then(|path| path.parent()) {
        return root.to_owned();
    }
    manifest_dir
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
