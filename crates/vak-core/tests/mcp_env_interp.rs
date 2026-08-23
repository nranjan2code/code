#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

// ${VAR} interpolation for MCP server env values resolves through the same
// secret path as provider keys; unresolved references fail closed.

#[test]
fn literal_values_pass_through() {
    assert_eq!(
        vak_core::interpolate_env_var("plain"),
        Some("plain".to_string())
    );
}

#[test]
fn resolves_from_runtime_override() {
    vak_config::set_override("TEST_MCP_KEY", "abc123");
    assert_eq!(
        vak_core::interpolate_env_var("${TEST_MCP_KEY}"),
        Some("abc123".into())
    );
    assert_eq!(
        vak_core::interpolate_env_var("Bearer ${TEST_MCP_KEY}"),
        Some("Bearer abc123".into())
    );
    vak_config::clear_override("TEST_MCP_KEY");
}

#[test]
fn unresolved_reference_fails_closed() {
    vak_config::forget_dotenv_var("DEFINITELY_UNSET_MCP_VAR_9");
    assert_eq!(
        vak_core::interpolate_env_var("${DEFINITELY_UNSET_MCP_VAR_9}"),
        None::<String>
    );
}
