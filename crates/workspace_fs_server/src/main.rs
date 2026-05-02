#[tokio::main]
async fn main() -> anyhow::Result<()> {
    workspace_fs_server::http::server::run_from_env().await
}
