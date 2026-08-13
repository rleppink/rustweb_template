use sea_orm_migration::prelude::*;

// Lets you run the migrator as a CLI: `cargo run -p migration -- up`
#[tokio::main]
async fn main() {
    cli::run_cli(migration::Migrator).await;
}
