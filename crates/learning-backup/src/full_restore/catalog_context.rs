//! Only catalog deparsing uses this trusted context; payload execution does not.

pub(super) const PATH: &str = "pg_catalog,public";

pub(super) fn guard(assertion: &str, observation: &str) -> String {
    let Some(assertion_body) = block(assertion, "DO $kw$ ", " $kw$;\n") else {
        return format!("{observation}{assertion}");
    };
    let observation = if observation.is_empty() {
        String::new()
    } else if let Some(body) = block(observation, "DO $kw_c5$\n", " $kw_c5$;\n") {
        body
    } else {
        // Unexpected generated framing must retain the original strict check.
        return format!("{observation}{assertion}");
    };
    format!(
        "DO $kw_catalog$ DECLARE previous_path text; BEGIN\nprevious_path := pg_catalog.current_setting('search_path');\nPERFORM pg_catalog.set_config('search_path','{PATH}',true);\n{observation}{assertion_body}PERFORM pg_catalog.set_config('search_path',previous_path,true);\nEND $kw_catalog$;\n"
    )
}

fn block(sql: &str, prefix: &str, suffix: &str) -> Option<String> {
    Some(format!(
        "{};\n",
        sql.strip_prefix(prefix)?.strip_suffix(suffix)?
    ))
}

#[cfg(test)]
mod tests {
    use super::guard;

    const ASSERTION: &str = "DO $kw$ BEGIN IF NOT COALESCE((false), false) THEN RAISE EXCEPTION 'KW_C4_CATALOG'; END IF; END $kw$;\n";
    const OBSERVATION: &str = "DO $kw_c5$\nBEGIN /* TEST_ONLY_OBSERVATION */ NULL; END $kw_c5$;\n";

    #[test]
    fn c6_generated_guard_has_trusted_context_and_restores_before_completion() {
        let generated = guard(ASSERTION, "");
        let save = generated
            .find("pg_catalog.current_setting('search_path')")
            .expect("C6 context must save the original path");
        let set = generated
            .find("pg_catalog.set_config('search_path','pg_catalog,public',true)")
            .expect("C6 context must use the fixed trusted render path");
        let check = generated.find("KW_C4_CATALOG").unwrap();
        let restore = generated
            .find("pg_catalog.set_config('search_path',previous_path,true)")
            .expect("C6 context must restore the original path");
        assert!(save < set && set < check && check < restore);
        assert_eq!(generated.matches("KW_C4_CATALOG").count(), 1);
        assert!(!generated.contains("COMMIT"));
        assert!(!generated.contains("ROLLBACK"));
    }

    #[test]
    fn c6_guard_keeps_strict_assertion_and_has_no_receipt_or_normalization() {
        let generated = guard(ASSERTION, "");
        assert!(generated.contains("IF NOT COALESCE((false), false)"));
        assert!(generated.contains("RAISE EXCEPTION 'KW_C4_CATALOG'"));
        for forbidden in [
            "replace(",
            "|READY|",
            "|PRECOMMIT|",
            "RAISE WARNING",
            "WHEN OTHERS",
        ] {
            assert!(!generated.contains(forbidden));
        }
        assert_eq!(
            guard(ASSERTION, "UNEXPECTED_OBSERVATION"),
            format!("UNEXPECTED_OBSERVATION{ASSERTION}")
        );
    }

    #[test]
    fn c6_observation_stays_before_the_original_strict_rejection() {
        let generated = guard(ASSERTION, OBSERVATION);
        assert!(
            generated.find("TEST_ONLY_OBSERVATION").unwrap()
                < generated.find("KW_C4_CATALOG").unwrap()
        );
    }

    #[test]
    fn c6_observation_is_inside_the_same_trusted_render_context() {
        let generated = guard(ASSERTION, OBSERVATION);
        let set = generated
            .find("pg_catalog.set_config('search_path','pg_catalog,public',true)")
            .expect("C5 observation needs the check context too");
        let observation = generated.find("TEST_ONLY_OBSERVATION").unwrap();
        let restore = generated
            .find("pg_catalog.set_config('search_path',previous_path,true)")
            .expect("missing restore");
        assert!(set < observation && observation < restore);
    }
}
