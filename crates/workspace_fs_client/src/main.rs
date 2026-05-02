#[tokio::main]
async fn main() -> anyhow::Result<()> {
    workspace_fs_client::client_proxy_app::run_from_env().await
}
