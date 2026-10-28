use super::{Database, ModelPricingRow, NewModelPricing, UpdateModelPricing};

const PRICING_COLUMNS: &str = "id, model_name, is_advertised, is_free, price_per_1m_input_sats, \
    price_per_1m_output_sats, price_per_1m_cache_read_sats, price_per_1m_cache_write_sats, price_per_request_sats, \
    context_window, max_output_tokens, created_at, updated_at";

impl Database {
    pub async fn get_model_pricing(&self, model_name: &str) -> Result<Option<ModelPricingRow>, sqlx::Error> {
        sqlx::query_as::<_, ModelPricingRow>(&format!("SELECT {PRICING_COLUMNS} FROM model_pricing WHERE model_name = ?"))
            .bind(model_name)
            .fetch_optional(&self.pool)
            .await
    }

    async fn model_pricing_by_id(&self, id: i64) -> Result<ModelPricingRow, sqlx::Error> {
        sqlx::query_as::<_, ModelPricingRow>(&format!("SELECT {PRICING_COLUMNS} FROM model_pricing WHERE id = ?"))
            .bind(id)
            .fetch_one(&self.pool)
            .await
    }

    pub async fn list_model_pricings(&self) -> Result<Vec<ModelPricingRow>, sqlx::Error> {
        sqlx::query_as::<_, ModelPricingRow>(&format!("SELECT {PRICING_COLUMNS} FROM model_pricing ORDER BY model_name"))
            .fetch_all(&self.pool)
            .await
    }

    pub async fn create_model_pricing(&self, mp: NewModelPricing<'_>) -> Result<ModelPricingRow, sqlx::Error> {
        let id = sqlx::query(
            "INSERT INTO model_pricing (model_name, is_advertised, is_free, price_per_1m_input_sats, \
             price_per_1m_output_sats, price_per_1m_cache_read_sats, price_per_1m_cache_write_sats, \
             price_per_request_sats, context_window, max_output_tokens) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(mp.model_name)
        .bind(mp.is_advertised)
        .bind(mp.is_free)
        .bind(mp.price_per_1m_input_sats)
        .bind(mp.price_per_1m_output_sats)
        .bind(mp.price_per_1m_cache_read_sats)
        .bind(mp.price_per_1m_cache_write_sats)
        .bind(mp.price_per_request_sats)
        .bind(mp.context_window)
        .bind(mp.max_output_tokens)
        .execute(&self.pool)
        .await?
        .last_insert_id() as i64;
        self.model_pricing_by_id(id).await
    }

    pub async fn update_model_pricing(
        &self,
        model_name: &str,
        updates: UpdateModelPricing,
    ) -> Result<ModelPricingRow, sqlx::Error> {
        let current = self.get_model_pricing(model_name).await?.ok_or(sqlx::Error::RowNotFound)?;
        let merged = ModelPricingRow {
            is_advertised: updates.is_advertised.unwrap_or(current.is_advertised),
            is_free: updates.is_free.unwrap_or(current.is_free),
            price_per_1m_input_sats: updates.price_per_1m_input_sats.unwrap_or(current.price_per_1m_input_sats),
            price_per_1m_output_sats: updates.price_per_1m_output_sats.unwrap_or(current.price_per_1m_output_sats),
            price_per_1m_cache_read_sats: updates
                .price_per_1m_cache_read_sats
                .unwrap_or(current.price_per_1m_cache_read_sats),
            price_per_1m_cache_write_sats: updates
                .price_per_1m_cache_write_sats
                .unwrap_or(current.price_per_1m_cache_write_sats),
            price_per_request_sats: updates.price_per_request_sats.unwrap_or(current.price_per_request_sats),
            context_window: updates.context_window.unwrap_or(current.context_window),
            max_output_tokens: updates.max_output_tokens.unwrap_or(current.max_output_tokens),
            ..current
        };
        sqlx::query(
            "UPDATE model_pricing SET is_advertised = ?, is_free = ?, price_per_1m_input_sats = ?, \
             price_per_1m_output_sats = ?, price_per_1m_cache_read_sats = ?, price_per_1m_cache_write_sats = ?, \
             price_per_request_sats = ?, context_window = ?, max_output_tokens = ?, \
             updated_at = CURRENT_TIMESTAMP(3) WHERE model_name = ?",
        )
        .bind(merged.is_advertised)
        .bind(merged.is_free)
        .bind(merged.price_per_1m_input_sats)
        .bind(merged.price_per_1m_output_sats)
        .bind(merged.price_per_1m_cache_read_sats)
        .bind(merged.price_per_1m_cache_write_sats)
        .bind(merged.price_per_request_sats)
        .bind(merged.context_window)
        .bind(merged.max_output_tokens)
        .bind(model_name)
        .execute(&self.pool)
        .await?;
        self.get_model_pricing(model_name).await?.ok_or(sqlx::Error::RowNotFound)
    }

    pub async fn delete_model_pricing(&self, model_name: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM model_pricing WHERE model_name = ?")
            .bind(model_name)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}
