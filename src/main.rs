#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    gigi::run_cli().await
}

#[cfg(not(feature = "ssr"))]
fn main() {}
