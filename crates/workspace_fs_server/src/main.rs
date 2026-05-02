#[tokio::main]
async fn main() -> anyhow::Result<()> {
    workspace_fs_server::server_app::run_from_env().await
}
