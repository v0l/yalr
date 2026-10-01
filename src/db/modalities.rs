use super::{Database, RoutingConfig};
use crate::router::Modality;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeclaredModalities {
    pub input: Option<Vec<Modality>>,
    pub output: Option<Vec<Modality>>,
}

impl DeclaredModalities {
    pub fn parse_list(names: &[String]) -> Result<Option<Vec<Modality>>, String> {
        let mut parsed = Vec::new();
        for name in names {
            let modality = Modality::parse(&name.trim().to_ascii_lowercase())
                .ok_or_else(|| format!("Unknown modality `{name}`, expected text, image, audio or video"))?;
            if !parsed.contains(&modality) {
                parsed.push(modality);
            }
        }
        Ok((!parsed.is_empty()).then_some(parsed))
    }

    pub fn names(list: &Option<Vec<Modality>>) -> Option<Vec<String>> {
        list.as_ref()
            .map(|l| l.iter().map(|m| m.as_str().to_string()).collect())
    }

    pub fn rejection(&self, input: &[Modality], output: &[Modality]) -> Option<String> {
        let lacks = |declared: &Option<Vec<Modality>>, wanted: &[Modality]| {
            declared
                .as_ref()
                .and_then(|d| wanted.iter().find(|m| !d.contains(m)).copied())
        };
        if let Some(m) = lacks(&self.input, input) {
            return Some(format!("does not accept {} input", m.as_str()));
        }
        lacks(&self.output, output).map(|m| format!("does not produce {} output", m.as_str()))
    }
}

fn decode(column: Option<&str>) -> Option<Vec<Modality>> {
    let names: Vec<String> = serde_json::from_str(column?).ok()?;
    let parsed: Vec<Modality> = names.iter().filter_map(|n| Modality::parse(n)).collect();
    (!parsed.is_empty()).then_some(parsed)
}

fn encode(list: &Option<Vec<Modality>>) -> Option<String> {
    DeclaredModalities::names(list).and_then(|names| serde_json::to_string(&names).ok())
}

impl RoutingConfig {
    pub fn declared_modalities(&self) -> DeclaredModalities {
        DeclaredModalities {
            input: decode(self.input_modalities.as_deref()),
            output: decode(self.output_modalities.as_deref()),
        }
    }
}

impl Database {
    pub async fn set_routing_config_modalities(
        &self,
        id: i64,
        modalities: &DeclaredModalities,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE routing_config SET input_modalities = ?, output_modalities = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        )
        .bind(encode(&modalities.input))
        .bind(encode(&modalities.output))
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::NewRoutingConfig;

    async fn config(db: &Database) -> RoutingConfig {
        db.create_routing_config(NewRoutingConfig {
            name: "painter".into(),
            strategy: "round_robin".into(),
            health_check_enabled: false,
            health_check_interval_seconds: 30,
            health_check_timeout_seconds: 5,
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn new_config_declares_nothing() {
        let db = Database::new("sqlite::memory:").await.unwrap();
        let rc = config(&db).await;
        assert_eq!(rc.declared_modalities(), DeclaredModalities::default());
        assert_eq!(rc.declared_modalities().rejection(&[Modality::Image], &[Modality::Audio]), None);
    }

    #[tokio::test]
    async fn modalities_round_trip_through_the_database() {
        let db = Database::new("sqlite::memory:").await.unwrap();
        let rc = config(&db).await;
        let declared = DeclaredModalities {
            input: Some(vec![Modality::Text, Modality::Image]),
            output: Some(vec![Modality::Image]),
        };
        db.set_routing_config_modalities(rc.id, &declared).await.unwrap();

        let stored = db.get_routing_config(rc.id).await.unwrap().unwrap();
        assert_eq!(stored.input_modalities.as_deref(), Some(r#"["text","image"]"#));
        assert_eq!(stored.declared_modalities(), declared);

        db.set_routing_config_modalities(rc.id, &DeclaredModalities::default())
            .await
            .unwrap();
        let cleared = db.get_routing_config(rc.id).await.unwrap().unwrap();
        assert_eq!(cleared.input_modalities, None);
    }

    #[test]
    fn parse_list_validates_and_dedupes() {
        let names = vec!["Text".to_string(), "image".into(), "text".into()];
        assert_eq!(
            DeclaredModalities::parse_list(&names).unwrap(),
            Some(vec![Modality::Text, Modality::Image])
        );
        assert_eq!(DeclaredModalities::parse_list(&[]).unwrap(), None);
        assert!(DeclaredModalities::parse_list(&["smell".to_string()]).is_err());
    }

    #[test]
    fn rejection_names_the_missing_modality() {
        let chat = DeclaredModalities {
            input: Some(vec![Modality::Text]),
            output: Some(vec![Modality::Text]),
        };
        assert_eq!(
            chat.rejection(&[Modality::Image], &[]).as_deref(),
            Some("does not accept image input")
        );
        assert_eq!(
            chat.rejection(&[], &[Modality::Image]).as_deref(),
            Some("does not produce image output")
        );
        assert_eq!(chat.rejection(&[Modality::Text], &[Modality::Text]), None);

        let input_only = DeclaredModalities {
            input: Some(vec![Modality::Text]),
            output: None,
        };
        assert_eq!(input_only.rejection(&[], &[Modality::Image]), None);
    }
}
