//! Old columns and rows only; shared by the producer and upgrade verifier.
//! The dedicated fixture database contains exactly one actor and one space.
use serde_json::{Value, json};
use sqlx::PgPool;

fn identifier(s: &str) -> String {
    assert!(
        !s.is_empty()
            && s.bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
    );
    format!("\"{s}\"")
}

pub async fn snapshot(pool: &PgPool, layout: Option<&Value>) -> Value {
    let columns: Vec<(String, Vec<String>)> = if let Some(layout) = layout {
        layout
            .as_object()
            .unwrap()
            .iter()
            .map(|(table, data)| {
                (
                    table.clone(),
                    serde_json::from_value(data["columns"].clone()).unwrap(),
                )
            })
            .collect()
    } else {
        sqlx::query_as("SELECT table_name::text,array_agg(column_name::text ORDER BY ordinal_position) FROM information_schema.columns WHERE table_schema='public' AND table_name IN (SELECT tablename FROM pg_tables WHERE schemaname='public') AND table_name<>'_sqlx_migrations' GROUP BY table_name ORDER BY table_name")
            .fetch_all(pool).await.unwrap()
    };
    let mut result = json!({});
    for (table, columns) in columns {
        let list = columns
            .iter()
            .map(|c| identifier(c))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT to_jsonb(r) FROM (SELECT {list} FROM public.{}) r ORDER BY to_jsonb(r)::text",
            identifier(&table)
        );
        let rows: Vec<Value> = sqlx::query_scalar(&sql).fetch_all(pool).await.unwrap();
        result[&table] = json!({"columns":columns,"count":rows.len(),"rows":rows});
    }
    result
}
