use super::Database;

mod access;
mod gateway_cutover;
mod model_providers;
mod payouts;
mod providers;
mod routing;

async fn setup_test_db() -> Database {
    Database::test().await
}
