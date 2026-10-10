//! Test-only SQL for one bounded, content-free warning before strict rejection.

pub(super) fn sql(nonce: Option<&str>, catalog_sql: &str, expected_json: &str) -> String {
    let Some(nonce) = nonce.filter(|nonce| {
        nonce.len() == 32
            && nonce
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }) else {
        return String::new();
    };
    // The serde JSON is trusted, but even a dollar tag inside a JSON string
    // must not terminate this SQL block. JSON decodes this escape identically.
    let expected_json = expected_json.replace('$', "\\u0024").replace('\'', "''");
    let catalog_sql = catalog_sql.trim_end_matches(';');
    format!(
        r#"DO $kw_c5$
DECLARE actual jsonb; wanted jsonb; summary jsonb; key text; av jsonb; ev jsonb; path text; message text;
BEGIN
 BEGIN
  actual := ({catalog_sql})::jsonb;
  wanted := '{expected_json}'::jsonb;
  IF actual IS DISTINCT FROM wanted THEN
   summary := '{{}}'::jsonb;
   FOREACH key IN ARRAY ARRAY['database','schemas','relations','column_acl','functions','roles','memberships','default_acl','constraints','triggers'] LOOP
    av := actual->key; ev := wanted->key;
    summary := summary || pg_catalog.jsonb_build_object(key,pg_catalog.jsonb_build_object(
     'same',av IS NOT DISTINCT FROM ev,
     'actual_type',COALESCE(pg_catalog.jsonb_typeof(av),'missing'),
     'expected_type',COALESCE(pg_catalog.jsonb_typeof(ev),'missing'),
     'actual_count',CASE COALESCE(pg_catalog.jsonb_typeof(av),'missing') WHEN 'array' THEN pg_catalog.jsonb_array_length(av) WHEN 'object' THEN (SELECT count(*) FROM pg_catalog.jsonb_object_keys(av)) WHEN 'null' THEN 0 WHEN 'missing' THEN 0 ELSE 1 END,
     'expected_count',CASE COALESCE(pg_catalog.jsonb_typeof(ev),'missing') WHEN 'array' THEN pg_catalog.jsonb_array_length(ev) WHEN 'object' THEN (SELECT count(*) FROM pg_catalog.jsonb_object_keys(ev)) WHEN 'null' THEN 0 WHEN 'missing' THEN 0 ELSE 1 END,
     'actual_hash',pg_catalog.md5(COALESCE(av::text,'')),
     'expected_hash',pg_catalog.md5(COALESCE(ev::text,''))));
   END LOOP;
   path := pg_catalog.current_setting('search_path');
   message := 'KW_C4_CATALOG_OBSERVATION|' || pg_catalog.jsonb_build_object(
    'scope','TEST_OBSERVATION_NOT_AUTHORITY','nonce','{nonce}',
    'pid',pg_catalog.pg_backend_pid(),
    'start',(SELECT (EXTRACT(EPOCH FROM a.backend_start)*1000000)::bigint FROM pg_catalog.pg_stat_activity a WHERE a.pid=pg_catalog.pg_backend_pid()),
    'xid',pg_catalog.pg_current_xact_id()::text,
    'search_path_empty',path='',
    'search_path_public','public'=ANY(pg_catalog.current_schemas(false)),
    'search_path_hash',pg_catalog.md5(path),'keys',summary)::text;
   IF pg_catalog.octet_length(message) > 3072 THEN
    message := 'KW_C4_CATALOG_OBSERVATION_CAPACITY|{nonce}';
   END IF;
   IF pg_catalog.octet_length(message) <= 3072 THEN
    RAISE WARNING '%', message;
   END IF;
  END IF;
 EXCEPTION
  WHEN query_canceled THEN NULL;
  WHEN assert_failure THEN NULL;
  WHEN OTHERS THEN NULL;
 END;
END $kw_c5$;
"#
    )
}

#[cfg(test)]
mod tests {
    use super::sql;

    const NONCE: &str = "abababababababababababababababab";
    const QUERY: &str = "SELECT json_build_object('database', json_build_object('owner', 'admin'))";

    #[test]
    fn c5_none_generates_no_statement_even_with_invalid_inputs() {
        assert_eq!(sql(None, "not SQL", "not JSON"), "");
    }

    #[test]
    fn c5_invalid_nonce_cannot_generate_a_statement() {
        for nonce in [
            "",
            "ABABABABABABABABABABABABABABABAB",
            "short",
            "x';SELECT 1;--",
        ] {
            assert_eq!(sql(Some(nonce), QUERY, "{}"), "");
        }
    }

    #[test]
    fn c5_selected_observation_reads_current_writer_and_escapes_expected_literal() {
        let generated = sql(Some(NONCE), QUERY, r#"{"database":{"owner":"O'Brien"}}"#);
        assert!(
            generated.contains("actual := (SELECT json_build_object("),
            "missing actual catalog read"
        );
        assert!(
            generated.contains("O''Brien"),
            "expected JSON must remain one SQL literal"
        );
        for actual in [
            "pg_catalog.pg_backend_pid()",
            "a.backend_start",
            "pg_catalog.pg_current_xact_id()::text",
            NONCE,
        ] {
            assert!(
                generated.contains(actual),
                "missing current writer identity: {actual}"
            );
        }
    }

    #[test]
    fn c5_matching_catalog_is_quiet_and_diagnostic_errors_are_local() {
        let generated = sql(Some(NONCE), QUERY, "{}");
        let read = generated.find("actual :=").expect("missing catalog read");
        let mismatch = generated
            .find("IF actual IS DISTINCT FROM wanted THEN")
            .expect("missing mismatch guard");
        let warning = generated
            .find("RAISE WARNING '%', message;")
            .expect("missing stderr warning");
        let catch = generated
            .find("WHEN OTHERS THEN NULL;")
            .expect("missing local exception boundary");
        assert!(read < mismatch && mismatch < warning && warning < catch);
        assert_eq!(generated.matches("RAISE WARNING").count(), 1);
        assert!(!generated.contains("RAISE NOTICE"));
        assert!(!generated.contains("RAISE EXCEPTION"));
    }

    #[test]
    fn c5_only_closed_keys_and_safe_summary_fields_can_reach_warning() {
        let generated = sql(Some(NONCE), QUERY, r#"{"secret":"PRIVATE_VALUE"}"#);
        assert!(generated.contains("ARRAY['database','schemas','relations','column_acl','functions','roles','memberships','default_acl','constraints','triggers']"), "catalog keys must be closed");
        for field in [
            "'same'",
            "'actual_type'",
            "'expected_type'",
            "'actual_count'",
            "'expected_count'",
            "'actual_hash'",
            "'expected_hash'",
        ] {
            assert!(
                generated.contains(field),
                "missing safe summary field: {field}"
            );
        }
        let message = generated
            .split("message :=")
            .nth(1)
            .expect("missing bounded emission");
        assert!(!message.contains("PRIVATE_VALUE"));
        assert!(!message.contains("actual::text"));
        assert!(!message.contains("wanted::text"));
        assert!(!message.contains("av::text"));
        assert!(!message.contains("ev::text"));
        assert!(generated.contains("pg_catalog.md5(COALESCE(av::text,''))"));
        assert!(generated.contains("pg_catalog.md5(COALESCE(ev::text,''))"));
    }

    #[test]
    fn c5_metadata_budget_leaves_space_for_native_warning_context_and_original_error() {
        let generated = sql(Some(NONCE), QUERY, "{}");
        assert!(
            generated.contains("pg_catalog.octet_length(message) > 3072"),
            "missing metadata cap"
        );
        assert!(
            generated.contains("KW_C4_CATALOG_OBSERVATION_CAPACITY"),
            "capacity must report only a fixed token"
        );
        assert!(
            generated.contains("pg_catalog.octet_length(message) <= 3072"),
            "emission must remain bounded"
        );
        assert_eq!(generated.matches("RAISE WARNING").count(), 1);
    }

    #[test]
    fn c5_search_path_is_observed_without_guc_or_privilege_changes() {
        let generated = sql(Some(NONCE), QUERY, "{}");
        for observation in [
            "pg_catalog.current_setting('search_path')",
            "pg_catalog.current_schemas(false)",
            "'search_path_empty'",
            "'search_path_public'",
            "'search_path_hash'",
        ] {
            assert!(
                generated.contains(observation),
                "missing setting observation: {observation}"
            );
        }
        for forbidden in [
            "set_config",
            "SET search_path",
            "SET ROLE",
            "GRANT ",
            "REVOKE ",
            "COMMIT",
            "ROLLBACK",
            "|READY|",
            "|PRECOMMIT|",
        ] {
            assert!(
                !generated.contains(forbidden),
                "diagnostic has a side effect or stdout receipt: {forbidden}"
            );
        }
    }

    #[test]
    fn c5_missing_and_scalar_values_have_nonthrowing_type_and_count_metadata() {
        let generated = sql(Some(NONCE), QUERY, "{}");
        assert!(generated.contains("COALESCE(pg_catalog.jsonb_typeof(av),'missing')"));
        assert!(generated.contains("COALESCE(pg_catalog.jsonb_typeof(ev),'missing')"));
        assert!(generated.contains("WHEN 'array' THEN pg_catalog.jsonb_array_length(av)"));
        assert!(generated.contains(
            "WHEN 'object' THEN (SELECT count(*) FROM pg_catalog.jsonb_object_keys(av))"
        ));
        assert!(generated.contains("WHEN 'null' THEN 0 WHEN 'missing' THEN 0 ELSE 1 END"));
        assert!(generated.contains("av IS NOT DISTINCT FROM ev"));
    }

    #[test]
    fn c5_cancellation_excluded_from_others_is_also_contained_in_diagnostic() {
        let generated = sql(Some(NONCE), QUERY, "{}");
        assert!(
            generated.contains("WHEN query_canceled THEN NULL;"),
            "diagnostic cancellation must not prevent the original check"
        );
        assert!(generated.contains("WHEN assert_failure THEN NULL;"));
    }

    #[test]
    fn c5_json_dollar_tags_cannot_terminate_the_diagnostic_block() {
        let generated = sql(Some(NONCE), QUERY, r#"{"definition":"$kw_c5$ \\ $kw$"}"#);
        assert_eq!(generated.matches("$kw_c5$").count(), 2);
        assert!(generated.contains("\\u0024kw_c5\\u0024"));
        assert!(generated.contains("\\u0024kw\\u0024"));
    }
}
