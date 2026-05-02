#[tokio::main]
async fn main() -> anyhow::Result<()> {
    workspace_fs_client::runtime::app::run_from_env().await
}
