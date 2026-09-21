mod support;

#[tokio::test]
async fn versioned_reading_and_release_children_are_runtime_immutable() {
    let r = support::TestRig::from_env().await;
    let columns: Vec<String> = sqlx::query_scalar("SELECT table_name || '.' || column_name FROM information_schema.columns WHERE table_schema='public' AND (table_name='reading_view_revision' OR table_name='release') AND column_name IN ('contract_version','manifest_sha256') ORDER BY 1")
        .fetch_all(&r.admin_pool).await.unwrap();
    assert_eq!(
        columns,
        [
            "reading_view_revision.contract_version",
            "release.contract_version",
            "release.manifest_sha256"
        ]
    );
    for table in [
        "reading_relation_selection",
        "reading_epistemic_selection",
        "release_reading",
        "release_manifest_object",
        "release_manifest_composition",
    ] {
        let rights: (bool, bool, bool, bool) = sqlx::query_as("SELECT has_table_privilege(current_user,$1,'SELECT'),has_table_privilege(current_user,$1,'INSERT'),has_table_privilege(current_user,$1,'UPDATE'),has_table_privilege(current_user,$1,'DELETE')")
            .bind(table).fetch_one(&r.runtime_pool).await.unwrap();
        assert_eq!(rights, (true, true, false, false), "{table}");
    }
}
