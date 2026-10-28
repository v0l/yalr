use super::Database;

mod access;
mod model_providers;
mod providers;
mod routing;

async fn setup_test_db() -> Database {
    Database::test().await
}
